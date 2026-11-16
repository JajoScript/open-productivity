// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for financial goals: a target amount reached by the balance of the goal's own reserve.

use chrono::NaiveDate;
use rusqlite::Connection;

use super::*;
use crate::db::test_conn;
use crate::finances::{
    AccountKind, create_account, create_card, delete_account, delete_reserve, list_moves,
    list_reserves, reserve_money, set_main_currency, withdraw_from_reserve,
};

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
}

fn setup() -> (Connection, AccountId) {
    let conn = test_conn();
    set_main_currency(&conn, "CLP").unwrap();
    let a = create_account(
        &conn,
        "Cta",
        AccountKind::Checking,
        "",
        "CLP",
        "500.000",
        None,
    )
    .unwrap();
    (conn, a)
}

fn money(title: &str, target: &str, account: AccountId) -> GoalDraft {
    GoalDraft {
        title: title.into(),
        financial: Some(Financial {
            target: target.into(),
            account,
        }),
        ..GoalDraft::default()
    }
}

fn fin(conn: &Connection, id: GoalId) -> FinancialProgress {
    get(conn, id, today()).unwrap().financial.unwrap()
}

fn put(conn: &Connection, id: GoalId, amount: &str) {
    reserve_money(conn, fin(conn, id).reserve, amount, today(), "").unwrap();
}

fn take(conn: &Connection, id: GoalId, amount: &str) {
    withdraw_from_reserve(conn, fin(conn, id).reserve, amount, today(), "").unwrap();
}

#[test]
fn create_makes_its_empty_reserve_in_the_account() {
    let (conn, a) = setup();
    let id = create(&conn, GoalKind::Goal, &money(" Viaje ", "100.000", a)).unwrap();
    let g = get(&conn, id, today()).unwrap();
    let f = g.financial.unwrap();
    assert_eq!((f.target, f.reserved, f.percent), (100_000, 0, 0));
    assert_eq!(
        (f.currency.code, f.account, f.account_name.as_str()),
        ("CLP", a, "Cta")
    );
    assert_eq!((g.numeric, g.progress), (None, Some(0)));
    let reserves = list_reserves(&conn).unwrap();
    assert_eq!(reserves.len(), 1);
    assert_eq!(
        (reserves[0].id, reserves[0].name.as_str()),
        (f.reserve, "Viaje")
    );
    // It belongs to the goal: not deletable from Finances, and it keeps the account in use.
    assert!(reserves[0].in_use);
    assert!(matches!(
        delete_reserve(&conn, f.reserve),
        Err(FinanceError::ReserveInUse)
    ));
    assert!(matches!(
        delete_account(&conn, a),
        Err(FinanceError::AccountInUse)
    ));
}

#[test]
fn the_target_is_in_the_accounts_currency() {
    let (conn, _) = setup();
    let usd = create_account(
        &conn,
        "Dólares",
        AccountKind::Savings,
        "",
        "USD",
        "0",
        Some("950"),
    )
    .unwrap();
    let id = create(&conn, GoalKind::Goal, &money("Mac", "1.200,50", usd)).unwrap();
    let f = fin(&conn, id);
    assert_eq!((f.target, f.currency.code), (120_050, "USD"));
}

#[test]
fn create_validates_and_leaves_nothing_behind() {
    let (conn, a) = setup();
    let both = GoalDraft {
        numeric: Some(Numeric {
            target: 10,
            unit: String::new(),
        }),
        ..money("X", "100", a)
    };
    assert!(matches!(
        create(&conn, GoalKind::Goal, &both),
        Err(GoalError::TwoMeasures)
    ));
    assert!(matches!(
        create(&conn, GoalKind::Project, &money("X", "100", a)),
        Err(GoalError::ProjectMeasure)
    ));
    for bad in ["", "0", "abc", "-5"] {
        assert!(matches!(
            create(&conn, GoalKind::Goal, &money("X", bad, a)),
            Err(GoalError::BadAmount)
        ));
    }
    assert!(matches!(
        create(&conn, GoalKind::Goal, &money("X", "100", AccountId(99))),
        Err(GoalError::Finance(FinanceError::AccountNotFound))
    ));
    let card = create_card(&conn, "Visa", "", "CLP", "1000", 5, 20, None).unwrap();
    // The goal row is rolled back when its reserve is refused.
    assert!(matches!(
        create(&conn, GoalKind::Goal, &money("X", "100", card)),
        Err(GoalError::Finance(FinanceError::KindLocked))
    ));
    assert!(list(&conn, today()).unwrap().is_empty());
    assert!(list_reserves(&conn).unwrap().is_empty());
}

#[test]
fn progress_is_the_reserve_balance() {
    let (conn, a) = setup();
    let id = create(&conn, GoalKind::Goal, &money("Viaje", "100.000", a)).unwrap();
    put(&conn, id, "30.000");
    assert_eq!(fin(&conn, id).percent, 30);
    take(&conn, id, "10.000");
    let f = fin(&conn, id);
    assert_eq!((f.reserved, f.percent), (20_000, 20));
    assert_eq!(get(&conn, id, today()).unwrap().progress, Some(20));
    put(&conn, id, "150.000");
    let f = fin(&conn, id);
    assert_eq!((f.reserved, f.percent), (170_000, 100));
    assert_eq!(list_moves(&conn, f.reserve).unwrap().len(), 3);
}

#[test]
fn update_changes_the_target_and_renames_the_reserve() {
    let (conn, a) = setup();
    let id = create(&conn, GoalKind::Goal, &money("Viaje", "100.000", a)).unwrap();
    update(&conn, id, &money("Viaje a Chiloé", "200.000", a)).unwrap();
    assert_eq!(fin(&conn, id).target, 200_000);
    assert_eq!(list_reserves(&conn).unwrap()[0].name, "Viaje a Chiloé");
    assert!(matches!(
        update(&conn, id, &money("Viaje", "0", a)),
        Err(GoalError::BadAmount)
    ));
    let other = create_account(&conn, "Otra", AccountKind::Cash, "", "CLP", "0", None).unwrap();
    assert!(matches!(
        update(&conn, id, &money("Viaje", "100", other)),
        Err(GoalError::MeasureChanged)
    ));
    let bare = |title: &str| GoalDraft {
        title: title.into(),
        ..GoalDraft::default()
    };
    assert!(matches!(
        update(&conn, id, &bare("Viaje")),
        Err(GoalError::MeasureChanged)
    ));
    // A goal without a measure cannot gain one.
    let plain = create(&conn, GoalKind::Goal, &bare("Y")).unwrap();
    assert!(matches!(
        update(&conn, plain, &money("Y", "100", a)),
        Err(GoalError::MeasureChanged)
    ));
}

#[test]
fn completes_at_the_target_and_stays_completed_after_a_withdrawal() {
    let (conn, a) = setup();
    let id = create(&conn, GoalKind::Goal, &money("Viaje", "100.000", a)).unwrap();
    put(&conn, id, "99.999");
    assert_eq!(complete_reached(&conn).unwrap(), 0);
    put(&conn, id, "1");
    assert_eq!(complete_reached(&conn).unwrap(), 1);
    take(&conn, id, "50.000");
    assert_eq!(complete_reached(&conn).unwrap(), 0);
    let g = get(&conn, id, today()).unwrap();
    assert!(g.completed);
    assert_eq!(g.financial.unwrap().percent, 50);
}

#[test]
fn delete_needs_an_empty_reserve_and_removes_it() {
    let (conn, a) = setup();
    let id = create(&conn, GoalKind::Goal, &money("Viaje", "100.000", a)).unwrap();
    put(&conn, id, "10.000");
    assert!(matches!(delete(&conn, id), Err(GoalError::ReserveNotEmpty)));
    assert_eq!(list(&conn, today()).unwrap().len(), 1);
    take(&conn, id, "10.000");
    delete(&conn, id).unwrap();
    assert!(list(&conn, today()).unwrap().is_empty());
    assert!(list_reserves(&conn).unwrap().is_empty());
    let moves: i64 = conn
        .query_row("SELECT count(*) FROM finance_reserve_move", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(moves, 0);
    // With the reserve gone, the account can be deleted.
    delete_account(&conn, a).unwrap();
}
