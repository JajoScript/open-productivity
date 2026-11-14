// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for scheduled payments.

use chrono::NaiveDate;

use super::tests::account;
use super::*;
use crate::db;
use crate::recurrence::Frequency;

type Conn = rusqlite::Connection;

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

fn setup() -> (Conn, AccountId) {
    let conn = db::test_conn();
    set_main_currency(&conn, "CLP").unwrap();
    let a = account(&conn, "A", AccountKind::Checking, "", "1000").unwrap();
    (conn, a)
}

fn add(conn: &Conn, a: AccountId, first: &str, form: ScheduleForm) -> ScheduledId {
    create_scheduled(
        conn,
        TxKind::Expense,
        "Pago",
        a,
        "100",
        None,
        d(first),
        form,
    )
    .unwrap()
}

fn balance(conn: &Conn, a: AccountId) -> i64 {
    get_account(conn, a).unwrap().balance
}

fn confirm(conn: &Conn, id: ScheduledId, date: &str) {
    confirm_scheduled(conn, id, d(date), "100").unwrap();
}

fn state(conn: &Conn, id: ScheduledId) -> (NaiveDate, i64, bool) {
    let s = get_scheduled(conn, id).unwrap();
    (s.next_date, s.installments_paid, s.done)
}

fn txs(conn: &Conn, month: &str) -> Vec<Transaction> {
    list_month(conn, d(month), MovementFilter::All)
        .unwrap()
        .into_iter()
        .filter_map(|m| match m {
            Movement::Tx(t) => Some(t),
            Movement::Transfer(_) => None,
        })
        .collect()
}

#[test]
fn create_each_form_and_validate() {
    let (conn, a) = setup();
    let once = add(&conn, a, "2026-11-05", ScheduleForm::Once);
    let s = get_scheduled(&conn, once).unwrap();
    assert_eq!(
        (s.start_date, s.next_date),
        (d("2026-11-05"), d("2026-11-05"))
    );
    assert_eq!(
        (s.form, s.installments_paid, s.done),
        (ScheduleForm::Once, 0, false)
    );
    assert_eq!(
        (s.amount, s.description.as_str(), s.account_name.as_str()),
        (100, "Pago", "A")
    );
    let rec = add(
        &conn,
        a,
        "2026-11-05",
        ScheduleForm::Recurring(Frequency::Weekly),
    );
    assert_eq!(
        get_scheduled(&conn, rec).unwrap().form,
        ScheduleForm::Recurring(Frequency::Weekly)
    );
    let inst = add(&conn, a, "2026-11-05", ScheduleForm::Installments(12));
    assert_eq!(
        get_scheduled(&conn, inst).unwrap().form,
        ScheduleForm::Installments(12)
    );
    assert_eq!(list_scheduled(&conn).unwrap().len(), 3);

    let income = create_category(&conn, TxKind::Income, "Sueldo").unwrap();
    let old = create_category(&conn, TxKind::Expense, "Viejo").unwrap();
    set_category_archived(&conn, old, true).unwrap();
    let mk = |amount: &str, acc, cat, form| {
        create_scheduled(
            &conn,
            TxKind::Expense,
            "x",
            acc,
            amount,
            cat,
            d("2026-11-05"),
            form,
        )
    };
    for bad in ["", "0", "-5", "abc"] {
        assert!(matches!(
            mk(bad, a, None, ScheduleForm::Once),
            Err(FinanceError::InvalidAmount)
        ));
    }
    assert!(matches!(
        mk("5", AccountId(99), None, ScheduleForm::Once),
        Err(FinanceError::AccountNotFound)
    ));
    assert!(matches!(
        mk("5", a, Some(income), ScheduleForm::Once),
        Err(FinanceError::CategoryKindMismatch)
    ));
    assert!(matches!(
        mk("5", a, Some(old), ScheduleForm::Once),
        Err(FinanceError::CategoryArchived)
    ));
    assert!(matches!(
        mk("5", a, Some(CategoryId(99)), ScheduleForm::Once),
        Err(FinanceError::CategoryNotFound)
    ));
}

#[test]
fn invalid_forms_are_rejected() {
    let (conn, a) = setup();
    for form in [
        ScheduleForm::Installments(1),
        ScheduleForm::Installments(0),
        ScheduleForm::Installments(-3),
        ScheduleForm::Recurring(Frequency::Daily),
    ] {
        assert!(matches!(
            create_scheduled(
                &conn,
                TxKind::Expense,
                "x",
                a,
                "5",
                None,
                d("2026-11-05"),
                form
            ),
            Err(FinanceError::InvalidSchedule)
        ));
    }
    assert!(list_scheduled(&conn).unwrap().is_empty());
    let rec = add(
        &conn,
        a,
        "2026-11-05",
        ScheduleForm::Recurring(Frequency::Monthly),
    );
    assert!(matches!(
        update_scheduled(
            &conn,
            rec,
            "x",
            "5",
            a,
            None,
            d("2026-11-05"),
            Some(Frequency::Daily)
        ),
        Err(FinanceError::InvalidSchedule)
    ));
}

#[test]
fn archived_account_cannot_get_a_schedule() {
    let (conn, a) = setup();
    set_account_archived(&conn, a, true).unwrap();
    assert!(matches!(
        create_scheduled(
            &conn,
            TxKind::Expense,
            "x",
            a,
            "5",
            None,
            d("2026-11-05"),
            ScheduleForm::Once
        ),
        Err(FinanceError::AccountArchived)
    ));
}

#[test]
fn month_end_anchor_does_not_drift() {
    let (conn, a) = setup();
    let id = add(
        &conn,
        a,
        "2027-01-31",
        ScheduleForm::Recurring(Frequency::Monthly),
    );
    let mut seen = Vec::new();
    for _ in 0..4 {
        let (next, _, _) = state(&conn, id);
        confirm(&conn, id, &next.to_string());
        seen.push(state(&conn, id).0);
    }
    assert_eq!(
        seen,
        [
            d("2027-02-28"),
            d("2027-03-31"),
            d("2027-04-30"),
            d("2027-05-31")
        ]
    );

    let leap = add(
        &conn,
        a,
        "2028-01-31",
        ScheduleForm::Recurring(Frequency::Monthly),
    );
    skip_scheduled(&conn, leap).unwrap();
    assert_eq!(state(&conn, leap).0, d("2028-02-29"));
    skip_scheduled(&conn, leap).unwrap();
    assert_eq!(state(&conn, leap).0, d("2028-03-31"));

    // Installments keep the day too.
    let inst = add(&conn, a, "2027-01-31", ScheduleForm::Installments(3));
    confirm(&conn, inst, "2027-01-31");
    confirm(&conn, inst, "2027-02-28");
    assert_eq!(state(&conn, inst), (d("2027-03-31"), 2, false));
}

#[test]
fn yearly_on_a_leap_day() {
    let (conn, a) = setup();
    let id = add(
        &conn,
        a,
        "2028-02-29",
        ScheduleForm::Recurring(Frequency::Yearly),
    );
    let mut seen = Vec::new();
    for _ in 0..4 {
        skip_scheduled(&conn, id).unwrap();
        seen.push(state(&conn, id).0);
    }
    assert_eq!(
        seen,
        [
            d("2029-02-28"),
            d("2030-02-28"),
            d("2031-02-28"),
            d("2032-02-29")
        ]
    );
}

#[test]
fn weekly_crosses_the_year() {
    let (conn, a) = setup();
    let id = add(
        &conn,
        a,
        "2026-12-28",
        ScheduleForm::Recurring(Frequency::Weekly),
    );
    skip_scheduled(&conn, id).unwrap();
    assert_eq!(state(&conn, id).0, d("2027-01-04"));
}

#[test]
fn due_rows_weekly_and_limited_installments() {
    let (conn, a) = setup();
    add(
        &conn,
        a,
        "2026-11-02",
        ScheduleForm::Recurring(Frequency::Weekly),
    );
    let dates = |rows: Vec<DueRow>| rows.iter().map(|r| r.date.to_string()).collect::<Vec<_>>();
    let rows = due_in_month(&conn, d("2026-11-15"), d("2026-01-01")).unwrap();
    assert_eq!(
        dates(rows),
        [
            "2026-11-02",
            "2026-11-09",
            "2026-11-16",
            "2026-11-23",
            "2026-11-30"
        ]
    );
    // Before the first date there is nothing.
    assert!(
        due_in_month(&conn, d("2026-10-01"), d("2026-01-01"))
            .unwrap()
            .is_empty()
    );

    add(&conn, a, "2026-11-10", ScheduleForm::Installments(3));
    let total: usize = (1..=12)
        .map(|m| {
            let month = d(&format!("2027-{m:02}-01"));
            due_in_month(&conn, month, d("2026-01-01"))
                .unwrap()
                .iter()
                .filter(|r| matches!(r.kind, DueKind::Installment { .. }))
                .count()
        })
        .sum();
    assert_eq!(total, 1); // 2026-11-10, 2026-12-10 and 2027-01-10: only the last is in 2027
}

#[test]
fn confirm_recurring_creates_the_movement_and_advances() {
    let (conn, a) = setup();
    let cat = create_category(&conn, TxKind::Expense, "Casa").unwrap();
    let id = create_scheduled(
        &conn,
        TxKind::Expense,
        "Renta",
        a,
        "100",
        Some(cat),
        d("2026-11-05"),
        ScheduleForm::Recurring(Frequency::Monthly),
    )
    .unwrap();
    confirm(&conn, id, "2026-11-07");
    assert_eq!(balance(&conn, a), 900);
    let t = txs(&conn, "2026-11-01");
    assert_eq!(t.len(), 1);
    assert_eq!(
        (t[0].date, t[0].amount, t[0].kind),
        (d("2026-11-07"), 100, TxKind::Expense)
    );
    assert_eq!(
        (t[0].description.as_str(), t[0].category),
        ("Renta", Some(cat))
    );
    assert_eq!(
        t[0].scheduled,
        Some(ScheduledLink {
            id,
            installment: None,
            restorable: false
        })
    );
    assert_eq!(state(&conn, id), (d("2026-12-05"), 0, false));
}

#[test]
fn confirm_with_an_edited_date_and_amount() {
    let (conn, a) = setup();
    let id = add(
        &conn,
        a,
        "2026-11-05",
        ScheduleForm::Recurring(Frequency::Monthly),
    );
    confirm_scheduled(&conn, id, d("2026-11-20"), "1.250").unwrap();
    let t = txs(&conn, "2026-11-01");
    assert_eq!((t[0].date, t[0].amount), (d("2026-11-20"), 1250));
    assert_eq!(balance(&conn, a), -250);
    // The schedule keeps its own amount and next date, counted from the scheduled date.
    let s = get_scheduled(&conn, id).unwrap();
    assert_eq!((s.amount, s.next_date), (100, d("2026-12-05")));
}

#[test]
fn confirm_with_a_bad_amount_changes_nothing() {
    let (conn, a) = setup();
    let id = add(&conn, a, "2026-11-05", ScheduleForm::Installments(3));
    for bad in ["", "0", "-1", "x"] {
        assert!(matches!(
            confirm_scheduled(&conn, id, d("2026-11-05"), bad),
            Err(FinanceError::InvalidAmount)
        ));
    }
    assert_eq!(state(&conn, id), (d("2026-11-05"), 0, false));
    assert!(txs(&conn, "2026-11-01").is_empty());
    assert_eq!(balance(&conn, a), 1000);
}

#[test]
fn confirming_still_works_after_the_category_is_archived() {
    let (conn, a) = setup();
    let cat = create_category(&conn, TxKind::Expense, "Casa").unwrap();
    let id = create_scheduled(
        &conn,
        TxKind::Expense,
        "x",
        a,
        "5",
        Some(cat),
        d("2026-11-05"),
        ScheduleForm::Once,
    )
    .unwrap();
    assert!(matches!(
        delete_category(&conn, cat),
        Err(FinanceError::InUse)
    ));
    set_category_archived(&conn, cat, true).unwrap();
    confirm(&conn, id, "2026-11-05");
    assert_eq!(txs(&conn, "2026-11-01")[0].category, Some(cat));
}

#[test]
fn confirming_the_last_installment_finishes_it() {
    let (conn, a) = setup();
    let id = add(&conn, a, "2026-11-05", ScheduleForm::Installments(2));
    confirm(&conn, id, "2026-11-05");
    assert_eq!(state(&conn, id), (d("2026-12-05"), 1, false));
    confirm(&conn, id, "2026-12-05");
    assert_eq!(state(&conn, id), (d("2027-01-05"), 2, true));
    for r in [
        confirm_scheduled(&conn, id, d("2026-12-06"), "100"),
        skip_scheduled(&conn, id),
        update_scheduled(&conn, id, "x", "5", a, None, d("2026-12-06"), None),
    ] {
        assert!(matches!(r, Err(FinanceError::ScheduledDone)));
    }
    assert_eq!(balance(&conn, a), 800);
    assert!(
        due_in_month(&conn, d("2027-01-01"), d("2027-01-01"))
            .unwrap()
            .is_empty()
    );
    let t = txs(&conn, "2026-12-01");
    assert_eq!(t[0].scheduled.as_ref().unwrap().installment, Some((2, 2)));
    assert_eq!(
        txs(&conn, "2026-11-01")[0]
            .scheduled
            .as_ref()
            .unwrap()
            .installment,
        Some((1, 2))
    );
    assert!(matches!(
        confirm_scheduled(&conn, ScheduledId(99), d("2026-12-06"), "1"),
        Err(FinanceError::ScheduledNotFound)
    ));
}

#[test]
fn skipping_does_not_pay() {
    let (conn, a) = setup();
    let id = add(&conn, a, "2026-11-05", ScheduleForm::Installments(3));
    skip_scheduled(&conn, id).unwrap();
    assert_eq!(state(&conn, id), (d("2026-12-05"), 0, false));
    assert_eq!(balance(&conn, a), 1000);
    // Three installments are still left, now ending one month later.
    let last = (1..=3)
        .flat_map(|m| due_in_month(&conn, d(&format!("2027-{m:02}-01")), d("2026-11-01")).unwrap())
        .last()
        .unwrap();
    assert_eq!(last.date, d("2027-02-05"));
    assert_eq!(
        last.kind,
        DueKind::Installment {
            number: 3,
            total: 3
        }
    );

    let once = add(&conn, a, "2026-11-05", ScheduleForm::Once);
    skip_scheduled(&conn, once).unwrap();
    assert!(get_scheduled(&conn, once).unwrap().done);
    assert!(txs(&conn, "2026-11-01").is_empty());
}

#[test]
fn due_rows_count_overdue_only_in_the_current_month() {
    let (conn, a) = setup();
    add(&conn, a, "2026-09-20", ScheduleForm::Once);
    let inst = add(&conn, a, "2026-10-15", ScheduleForm::Installments(4));
    confirm(&conn, inst, "2026-10-15"); // paid 1, next 11-15
    add(
        &conn,
        a,
        "2026-11-30",
        ScheduleForm::Recurring(Frequency::Weekly),
    );

    let view = |rows: &[DueRow]| {
        rows.iter()
            .map(|r| (r.date.to_string(), r.is_next, r.kind, r.overdue))
            .collect::<Vec<_>>()
    };
    let today = d("2026-11-20");
    let now = due_in_month(&conn, d("2026-11-17"), today).unwrap(); // any day normalizes
    assert_eq!(
        view(&now),
        [
            ("2026-09-20".to_string(), true, DueKind::Once, true),
            (
                "2026-11-15".to_string(),
                true,
                DueKind::Installment {
                    number: 2,
                    total: 4
                },
                true
            ),
            (
                "2026-11-30".to_string(),
                true,
                DueKind::Recurring(Frequency::Weekly),
                false
            ),
        ]
    );
    // In another month the overdue one does not show; only real occurrences do.
    let dec = due_in_month(&conn, d("2026-12-01"), today).unwrap();
    let v = view(&dec);
    assert_eq!(
        v[0],
        (
            "2026-12-07".to_string(),
            false,
            DueKind::Recurring(Frequency::Weekly),
            false
        )
    );
    assert_eq!(
        v[2],
        (
            "2026-12-15".to_string(),
            false,
            DueKind::Installment {
                number: 3,
                total: 4
            },
            false
        )
    );
    assert_eq!(dec.len(), 5); // 4 weekly dates (7, 14, 21, 28) plus the installment
    assert!(dec.iter().all(|r| !r.is_next));
    // The overdue once is not in October either: it is not that month's occurrence.
    assert!(
        due_in_month(&conn, d("2026-10-01"), today)
            .unwrap()
            .is_empty()
    );
    // A past month with a real occurrence marks it overdue.
    let sep = due_in_month(&conn, d("2026-09-01"), today).unwrap();
    assert_eq!(
        view(&sep),
        [("2026-09-20".to_string(), true, DueKind::Once, true)]
    );
}

#[test]
fn due_rows_sort_by_date_then_description() {
    let (conn, a) = setup();
    let mk = |desc: &str| {
        create_scheduled(
            &conn,
            TxKind::Income,
            desc,
            a,
            "5",
            None,
            d("2026-11-10"),
            ScheduleForm::Once,
        )
        .unwrap()
    };
    mk("b");
    mk("a");
    add(&conn, a, "2026-11-01", ScheduleForm::Once);
    let names: Vec<_> = due_in_month(&conn, d("2026-11-01"), d("2026-11-01"))
        .unwrap()
        .into_iter()
        .map(|r| r.scheduled.description)
        .collect();
    assert_eq!(names, ["Pago", "a", "b"]);
}

#[test]
fn edit_and_delete() {
    let (conn, a) = setup();
    let cat = create_category(&conn, TxKind::Expense, "Casa").unwrap();
    let id = add(
        &conn,
        a,
        "2026-11-30",
        ScheduleForm::Recurring(Frequency::Monthly),
    );
    update_scheduled(
        &conn,
        id,
        " Renta ",
        "250",
        a,
        Some(cat),
        d("2026-11-15"),
        Some(Frequency::Yearly),
    )
    .unwrap();
    let s = get_scheduled(&conn, id).unwrap();
    assert_eq!(
        (
            s.description.as_str(),
            s.amount,
            s.next_date,
            s.start_date,
            s.form
        ),
        (
            "Renta",
            250,
            d("2026-11-15"),
            d("2026-11-15"),
            ScheduleForm::Recurring(Frequency::Yearly)
        )
    );
    assert_eq!(s.category_name.as_deref(), Some("Casa"));
    // Same next date keeps the anchor.
    let m = add(
        &conn,
        a,
        "2026-01-31",
        ScheduleForm::Recurring(Frequency::Monthly),
    );
    skip_scheduled(&conn, m).unwrap(); // next 02-28, anchor 31
    update_scheduled(
        &conn,
        m,
        "x",
        "5",
        a,
        None,
        d("2026-02-28"),
        Some(Frequency::Monthly),
    )
    .unwrap();
    assert_eq!(get_scheduled(&conn, m).unwrap().start_date, d("2026-01-31"));
    skip_scheduled(&conn, m).unwrap();
    assert_eq!(state(&conn, m).0, d("2026-03-31"));

    // A recurring payment needs its frequency; the others refuse one.
    assert!(matches!(
        update_scheduled(&conn, id, "x", "1", a, None, d("2026-11-15"), None),
        Err(FinanceError::InvalidSchedule)
    ));
    let once = add(&conn, a, "2026-11-30", ScheduleForm::Once);
    assert!(matches!(
        update_scheduled(
            &conn,
            once,
            "x",
            "1",
            a,
            None,
            d("2026-11-15"),
            Some(Frequency::Weekly)
        ),
        Err(FinanceError::InvalidSchedule)
    ));
    assert!(matches!(
        update_scheduled(&conn, once, "x", "0", a, None, d("2026-11-15"), None),
        Err(FinanceError::InvalidAmount)
    ));
    let income = create_category(&conn, TxKind::Income, "Sueldo").unwrap();
    assert!(matches!(
        update_scheduled(
            &conn,
            once,
            "x",
            "1",
            a,
            Some(income),
            d("2026-11-15"),
            None
        ),
        Err(FinanceError::CategoryKindMismatch)
    ));
    // A bad edit changes nothing.
    assert_eq!(
        get_scheduled(&conn, once).unwrap().next_date,
        d("2026-11-30")
    );

    confirm(&conn, id, "2026-11-15");
    delete_scheduled(&conn, id).unwrap();
    assert!(matches!(
        get_scheduled(&conn, id),
        Err(FinanceError::ScheduledNotFound)
    ));
    assert!(matches!(
        delete_scheduled(&conn, id),
        Err(FinanceError::ScheduledNotFound)
    ));
    let t = txs(&conn, "2026-11-01");
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].scheduled, None);
    let null: Option<i64> = conn
        .query_row("SELECT scheduled_id FROM finance_transaction", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(null, None);
}

#[test]
fn editing_to_another_account_reads_the_amount_there() {
    let (conn, a) = setup();
    let usd = create_account(&conn, "U", AccountKind::Cash, "", "USD", "0", Some("900")).unwrap();
    let id = add(&conn, a, "2026-11-05", ScheduleForm::Once);
    update_scheduled(&conn, id, "x", "12,5", usd, None, d("2026-11-05"), None).unwrap();
    let s = get_scheduled(&conn, id).unwrap();
    assert_eq!((s.amount, s.currency.code), (1250, "USD"));
    set_account_archived(&conn, a, true).unwrap();
    assert!(matches!(
        update_scheduled(&conn, id, "x", "5", a, None, d("2026-11-05"), None),
        Err(FinanceError::AccountArchived)
    ));
}

#[test]
fn pending_schedules_block_archiving_the_account() {
    let (conn, a) = setup();
    let id = add(&conn, a, "2026-11-05", ScheduleForm::Installments(2));
    add(&conn, a, "2026-11-05", ScheduleForm::Once);
    assert!(matches!(
        set_account_archived(&conn, a, true),
        Err(FinanceError::AccountHasSchedules(2))
    ));
    assert!(!get_account(&conn, a).unwrap().archived);
    delete_scheduled(&conn, id).unwrap();
    assert!(matches!(
        set_account_archived(&conn, a, true),
        Err(FinanceError::AccountHasSchedules(1))
    ));
    let rest = list_scheduled(&conn).unwrap()[0].id;
    skip_scheduled(&conn, rest).unwrap(); // finishes the one-time payment
    set_account_archived(&conn, a, true).unwrap();
    // A finished schedule still references the account, so it cannot be deleted.
    set_account_archived(&conn, a, false).unwrap();
    assert!(matches!(
        delete_account(&conn, a),
        Err(FinanceError::AccountHasSchedules(1))
    ));
}

#[test]
fn schedules_lock_the_account_currency() {
    let (conn, a) = setup();
    add(&conn, a, "2026-11-05", ScheduleForm::Once);
    assert!(matches!(
        update_account(
            &conn,
            a,
            "A",
            AccountKind::Cash,
            "",
            "USD",
            "0",
            Some("900")
        ),
        Err(FinanceError::CurrencyLocked)
    ));
}

#[test]
fn restore_an_installment_returns_it_to_the_unpaid_ones() {
    let (conn, a) = setup();
    let id = add(&conn, a, "2026-11-05", ScheduleForm::Installments(2));
    confirm(&conn, id, "2026-11-05");
    confirm(&conn, id, "2026-12-05");
    assert_eq!(state(&conn, id), (d("2027-01-05"), 2, true));
    let t = txs(&conn, "2026-12-01").remove(0);
    assert!(t.scheduled.as_ref().unwrap().restorable);
    delete_transaction(&conn, t.id, true).unwrap();
    // Pending again, one installment paid, the next date unchanged.
    assert_eq!(state(&conn, id), (d("2027-01-05"), 1, false));
    assert_eq!(balance(&conn, a), 900);
    let rows = due_in_month(&conn, d("2027-01-01"), d("2026-12-20")).unwrap();
    assert_eq!(
        rows[0].kind,
        DueKind::Installment {
            number: 2,
            total: 2
        }
    );
    // It can be confirmed again, and the new movement is numbered 2.
    confirm(&conn, id, "2027-01-05");
    assert_eq!(state(&conn, id), (d("2027-02-05"), 2, true));
    assert_eq!(
        txs(&conn, "2027-01-01")[0]
            .scheduled
            .as_ref()
            .unwrap()
            .installment,
        Some((2, 2))
    );
}

#[test]
fn delete_without_restore_leaves_the_schedule() {
    let (conn, a) = setup();
    let id = add(&conn, a, "2026-11-05", ScheduleForm::Installments(2));
    confirm(&conn, id, "2026-11-05");
    let t = txs(&conn, "2026-11-01").remove(0);
    delete_transaction(&conn, t.id, false).unwrap();
    assert_eq!(state(&conn, id), (d("2026-12-05"), 1, false));
}

#[test]
fn restore_a_one_time_payment() {
    let (conn, a) = setup();
    let id = add(&conn, a, "2026-11-05", ScheduleForm::Once);
    confirm(&conn, id, "2026-11-06");
    assert!(get_scheduled(&conn, id).unwrap().done);
    let t = txs(&conn, "2026-11-01").remove(0);
    assert!(t.scheduled.as_ref().unwrap().restorable);
    delete_transaction(&conn, t.id, true).unwrap();
    // A one-time payment has no next occurrence, so its date never moved.
    let (next, paid, done) = state(&conn, id);
    assert_eq!((paid, done), (0, false));
    assert_eq!(next, d("2026-11-05"));
    assert_eq!(balance(&conn, a), 1000);
    confirm(&conn, id, "2026-11-07");
    assert!(get_scheduled(&conn, id).unwrap().done);
}

#[test]
fn restore_a_recurring_payment_does_nothing_extra() {
    let (conn, a) = setup();
    let id = add(
        &conn,
        a,
        "2026-11-05",
        ScheduleForm::Recurring(Frequency::Monthly),
    );
    confirm(&conn, id, "2026-11-05");
    let t = txs(&conn, "2026-11-01").remove(0);
    assert!(!t.scheduled.as_ref().unwrap().restorable);
    delete_transaction(&conn, t.id, true).unwrap();
    assert_eq!(state(&conn, id), (d("2026-12-05"), 0, false));
    assert!(txs(&conn, "2026-11-01").is_empty());
}

#[test]
fn restore_on_a_plain_or_missing_movement() {
    let (conn, a) = setup();
    let t = create_transaction(&conn, TxKind::Expense, a, "5", d("2026-11-05"), "", None).unwrap();
    delete_transaction(&conn, t, true).unwrap();
    assert!(matches!(
        delete_transaction(&conn, t, true),
        Err(FinanceError::TransactionNotFound)
    ));
}

#[test]
fn restore_after_the_schedule_was_deleted_just_deletes() {
    let (conn, a) = setup();
    let id = add(&conn, a, "2026-11-05", ScheduleForm::Once);
    confirm(&conn, id, "2026-11-05");
    delete_scheduled(&conn, id).unwrap();
    let t = txs(&conn, "2026-11-01").remove(0);
    assert!(t.scheduled.is_none());
    delete_transaction(&conn, t.id, true).unwrap();
    assert!(txs(&conn, "2026-11-01").is_empty());
}

#[test]
fn category_with_a_schedule_is_in_use() {
    let (conn, a) = setup();
    let cat = create_category(&conn, TxKind::Expense, "Casa").unwrap();
    create_scheduled(
        &conn,
        TxKind::Expense,
        "x",
        a,
        "5",
        Some(cat),
        d("2026-11-05"),
        ScheduleForm::Once,
    )
    .unwrap();
    assert!(get_category(&conn, cat).unwrap().in_use);
}
