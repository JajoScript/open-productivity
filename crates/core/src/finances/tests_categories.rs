// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for categories, their totals and their use in movements.

use chrono::NaiveDate;
use rusqlite::Connection;

use super::tests::account;
use super::*;
use crate::db;

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

fn setup() -> (Connection, AccountId) {
    let conn = db::test_conn();
    set_main_currency(&conn, "CLP").unwrap();
    let a = account(&conn, "A", AccountKind::Cash, "", "1000").unwrap();
    (conn, a)
}

fn names(conn: &Connection, kind: TxKind) -> Vec<(String, bool)> {
    list_categories(conn, kind)
        .unwrap()
        .into_iter()
        .map(|c| (c.name, c.archived))
        .collect()
}

fn spend(conn: &Connection, a: AccountId, date: &str, amount: &str, c: Option<CategoryId>) {
    create_transaction(conn, TxKind::Expense, a, amount, d(date), "", c).unwrap();
}

#[test]
fn create_trims_and_rejects_empty() {
    let (conn, _) = setup();
    let id = create_category(&conn, TxKind::Expense, "  Food ").unwrap();
    let c = get_category(&conn, id).unwrap();
    assert_eq!(c.name, "Food");
    assert_eq!(
        (c.kind, c.archived, c.in_use),
        (TxKind::Expense, false, false)
    );
    assert!(matches!(
        create_category(&conn, TxKind::Expense, "  "),
        Err(FinanceError::EmptyName)
    ));
}

#[test]
fn duplicates_ignore_case_and_archived_do_not_count() {
    let (conn, _) = setup();
    let food = create_category(&conn, TxKind::Expense, "Food").unwrap();
    assert!(matches!(
        create_category(&conn, TxKind::Expense, " fOOD "),
        Err(FinanceError::DuplicateName)
    ));
    // Same name in the other kind is fine.
    create_category(&conn, TxKind::Income, "Food").unwrap();
    set_category_archived(&conn, food, true).unwrap();
    let again = create_category(&conn, TxKind::Expense, "food").unwrap();
    // Restoring the old one now clashes with the active one.
    assert!(matches!(
        set_category_archived(&conn, food, false),
        Err(FinanceError::DuplicateName)
    ));
    set_category_archived(&conn, again, true).unwrap();
    set_category_archived(&conn, food, false).unwrap();
}

#[test]
fn rename_rules() {
    let (conn, _) = setup();
    let a = create_category(&conn, TxKind::Expense, "A").unwrap();
    let b = create_category(&conn, TxKind::Expense, "B").unwrap();
    rename_category(&conn, a, " a2 ").unwrap();
    assert_eq!(get_category(&conn, a).unwrap().name, "a2");
    // Changing only the case of its own name is allowed.
    rename_category(&conn, a, "A2").unwrap();
    assert!(matches!(
        rename_category(&conn, b, "a2"),
        Err(FinanceError::DuplicateName)
    ));
    assert!(matches!(
        rename_category(&conn, b, " "),
        Err(FinanceError::EmptyName)
    ));
    assert!(matches!(
        rename_category(&conn, CategoryId(99), "x"),
        Err(FinanceError::CategoryNotFound)
    ));
}

#[test]
fn archive_and_restore() {
    let (conn, _) = setup();
    let a = create_category(&conn, TxKind::Expense, "A").unwrap();
    set_category_archived(&conn, a, true).unwrap();
    assert!(get_category(&conn, a).unwrap().archived);
    set_category_archived(&conn, a, false).unwrap();
    assert!(!get_category(&conn, a).unwrap().archived);
    assert!(matches!(
        set_category_archived(&conn, CategoryId(99), true),
        Err(FinanceError::CategoryNotFound)
    ));
}

#[test]
fn delete_unused_and_in_use() {
    let (conn, a) = setup();
    let free = create_category(&conn, TxKind::Expense, "Free").unwrap();
    let used = create_category(&conn, TxKind::Expense, "Used").unwrap();
    spend(&conn, a, "2026-03-05", "10", Some(used));
    assert!(get_category(&conn, used).unwrap().in_use);
    assert!(matches!(
        delete_category(&conn, used),
        Err(FinanceError::InUse)
    ));
    delete_category(&conn, free).unwrap();
    assert!(matches!(
        delete_category(&conn, free),
        Err(FinanceError::CategoryNotFound)
    ));
    assert_eq!(names(&conn, TxKind::Expense), [("Used".to_string(), false)]);
}

#[test]
fn list_is_ordered_and_filtered_by_kind() {
    let (conn, _) = setup();
    for n in ["b", "C", "a"] {
        create_category(&conn, TxKind::Expense, n).unwrap();
    }
    let old = create_category(&conn, TxKind::Expense, "0old").unwrap();
    set_category_archived(&conn, old, true).unwrap();
    create_category(&conn, TxKind::Income, "Pay").unwrap();
    let want = [("a", false), ("b", false), ("C", false), ("0old", true)];
    assert_eq!(
        names(&conn, TxKind::Expense),
        want.map(|(n, a)| (n.to_string(), a))
    );
}

fn count(conn: &Connection) -> usize {
    names(conn, TxKind::Expense).len() + names(conn, TxKind::Income).len()
}

#[test]
fn suggested_are_created_once_in_one_transaction() {
    let (conn, _) = setup();
    assert!(!has_categories(&conn).unwrap());
    // A failing name aborts everything created before it.
    let r = create_suggested(&conn, |s| match s {
        Suggested::Salary => String::new(),
        s => format!("{s:?}"),
    });
    assert!(matches!(r, Err(FinanceError::EmptyName)));
    assert!(!has_categories(&conn).unwrap());

    create_suggested(&conn, |s| format!("{s:?}")).unwrap();
    assert!(has_categories(&conn).unwrap());
    assert_eq!(count(&conn), Suggested::ALL.len());
    assert!(Suggested::ALL.iter().any(|s| s.kind() == TxKind::Income));
    assert!(matches!(
        create_suggested(&conn, |s| format!("{s:?}2")),
        Err(FinanceError::CategoriesExist)
    ));
    assert_eq!(count(&conn), Suggested::ALL.len());
}

#[test]
fn suggested_fail_with_any_previous_category() {
    let (conn, _) = setup();
    let c = create_category(&conn, TxKind::Income, "X").unwrap();
    set_category_archived(&conn, c, true).unwrap();
    assert!(matches!(
        create_suggested(&conn, |s| format!("{s:?}")),
        Err(FinanceError::CategoriesExist)
    ));
    assert_eq!(count(&conn), 1);
}

#[test]
fn totals_by_category_for_the_month() {
    let (conn, a) = setup();
    let food = create_category(&conn, TxKind::Expense, "Food").unwrap();
    let fun = create_category(&conn, TxKind::Expense, "Fun").unwrap();
    let pay = create_category(&conn, TxKind::Income, "Pay").unwrap();
    create_category(&conn, TxKind::Expense, "Empty").unwrap();
    spend(&conn, a, "2026-02-28", "999", Some(food)); // before
    spend(&conn, a, "2026-03-01", "10", Some(food)); // first day
    spend(&conn, a, "2026-03-31", "15", Some(food)); // last day
    spend(&conn, a, "2026-04-01", "999", Some(food)); // after
    spend(&conn, a, "2026-03-10", "30", Some(fun));
    spend(&conn, a, "2026-03-11", "40", None);
    create_transaction(
        &conn,
        TxKind::Income,
        a,
        "500",
        d("2026-03-12"),
        "",
        Some(pay),
    )
    .unwrap();

    let t = category_totals(&conn, d("2026-03-17"), TxKind::Expense).unwrap();
    let got: Vec<_> = t
        .iter()
        .map(|c| (c.category, c.name.as_deref(), c.total))
        .collect();
    assert_eq!(
        got,
        [
            (None, None, 40),
            (Some(fun), Some("Fun"), 30),
            (Some(food), Some("Food"), 25)
        ]
    );
}

#[test]
fn totals_income_only_and_empty_month() {
    let (conn, a) = setup();
    let pay = create_category(&conn, TxKind::Income, "Pay").unwrap();
    spend(&conn, a, "2026-03-10", "30", None);
    create_transaction(
        &conn,
        TxKind::Income,
        a,
        "500",
        d("2026-03-12"),
        "",
        Some(pay),
    )
    .unwrap();
    let t = category_totals(&conn, d("2026-03-01"), TxKind::Income).unwrap();
    assert_eq!(t.len(), 1);
    assert_eq!((t[0].category, t[0].total), (Some(pay), 500));
    assert!(
        category_totals(&conn, d("2026-05-01"), TxKind::Income)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn transaction_category_rules() {
    let (conn, a) = setup();
    let day = d("2026-03-05");
    let food = create_category(&conn, TxKind::Expense, "Food").unwrap();
    let pay = create_category(&conn, TxKind::Income, "Pay").unwrap();
    let id = create_transaction(&conn, TxKind::Expense, a, "5", day, "", Some(food)).unwrap();
    let t = get_transaction(&conn, id).unwrap();
    assert_eq!(
        (t.category, t.category_name.as_deref()),
        (Some(food), Some("Food"))
    );

    assert!(matches!(
        create_transaction(&conn, TxKind::Expense, a, "5", day, "", Some(pay)),
        Err(FinanceError::CategoryKindMismatch)
    ));
    assert!(matches!(
        update_transaction(&conn, id, TxKind::Income, a, "5", day, "", Some(food)),
        Err(FinanceError::CategoryKindMismatch)
    ));
    assert!(matches!(
        create_transaction(
            &conn,
            TxKind::Expense,
            a,
            "5",
            day,
            "",
            Some(CategoryId(99))
        ),
        Err(FinanceError::CategoryNotFound)
    ));

    // Archived: kept when editing, refused when newly assigned.
    let other = create_category(&conn, TxKind::Expense, "Other").unwrap();
    set_category_archived(&conn, food, true).unwrap();
    set_category_archived(&conn, other, true).unwrap();
    update_transaction(&conn, id, TxKind::Expense, a, "7", day, "n", Some(food)).unwrap();
    assert_eq!(get_transaction(&conn, id).unwrap().amount, 7);
    assert!(matches!(
        update_transaction(&conn, id, TxKind::Expense, a, "7", day, "", Some(other)),
        Err(FinanceError::CategoryArchived)
    ));
    assert!(matches!(
        create_transaction(&conn, TxKind::Expense, a, "5", day, "", Some(food)),
        Err(FinanceError::CategoryArchived)
    ));
    // Removing the category is always allowed.
    update_transaction(&conn, id, TxKind::Expense, a, "7", day, "", None).unwrap();
    assert_eq!(get_transaction(&conn, id).unwrap().category, None);
}
