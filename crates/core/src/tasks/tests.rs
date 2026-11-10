// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for tasks.

use super::*;
use crate::{db, lists};

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

fn at(s: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(s).unwrap()
}

const NOW: &str = "2026-06-15T12:00:00+00:00";

#[test]
fn create_on_stores_the_exact_date_and_validates_the_title() {
    let conn = db::test_conn();
    for date in ["2020-01-31", "2026-11-08", "2031-02-28"] {
        let id = create_on(&conn, "  x ", d(date)).unwrap();
        let (title, due, list): (String, String, Option<i64>) = conn
            .query_row(
                "SELECT title, due_date, list_id FROM task WHERE id = ?1",
                [id.0],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((title.as_str(), due.as_str(), list), ("x", date, None));
    }
    assert!(matches!(
        create_on(&conn, " 	", d("2026-11-08")),
        Err(TaskError::EmptyTitle)
    ));
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM task", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 3);
}

fn mk(conn: &Connection, title: &str) -> Result<TaskId, TaskError> {
    create(conn, title, View::All, d("2026-06-15"))
}

fn ls(conn: &Connection) -> Vec<Task> {
    list(conn, View::All, at(NOW)).unwrap()
}

fn today_titles(conn: &Connection, now: &str) -> Vec<String> {
    view_titles(conn, View::Today, now)
}

fn view_titles(conn: &Connection, view: View, now: &str) -> Vec<String> {
    list(conn, view, at(now))
        .unwrap()
        .into_iter()
        .map(|t| t.title)
        .collect()
}

fn task(due: Option<&str>, done: bool) -> Task {
    Task {
        id: TaskId(1),
        title: String::new(),
        note: String::new(),
        done,
        due: due.map(d),
        start: None,
        end: None,
        list: None,
        priority: Priority::default(),
        repeat: None,
    }
}

fn titles(conn: &Connection) -> Vec<String> {
    ls(conn).into_iter().map(|t| t.title).collect()
}

fn done_at(conn: &Connection, id: TaskId) -> Option<String> {
    conn.query_row("SELECT done_at FROM task WHERE id = ?1", [id.0], |r| {
        r.get(0)
    })
    .unwrap()
}

#[test]
fn create_trims_title() {
    let conn = db::test_conn();
    mk(&conn, "  buy milk \n").unwrap();
    assert_eq!(titles(&conn), ["buy milk"]);
}

#[test]
fn create_in_inbox_has_no_list_or_date() {
    let conn = db::test_conn();
    create(&conn, "x", View::Inbox, d("2026-06-15")).unwrap();
    let tasks = list(&conn, View::Inbox, at(NOW)).unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].list.is_none() && tasks[0].due.is_none());
}

#[test]
fn empty_title_is_rejected_and_not_inserted() {
    let conn = db::test_conn();
    assert!(matches!(mk(&conn, ""), Err(TaskError::EmptyTitle)));
    assert!(matches!(mk(&conn, " \t\n"), Err(TaskError::EmptyTitle)));
    assert!(ls(&conn).is_empty());
}

#[test]
fn list_orders_pending_then_done_newest_first() {
    let conn = db::test_conn();
    let ids: Vec<_> = ["a", "b", "c", "d"]
        .iter()
        .map(|t| mk(&conn, t).unwrap())
        .collect();
    // Fixed timestamps: real ones could tie within a millisecond.
    for (id, at) in [
        (ids[0], "2026-01-01T00:00:01Z"),
        (ids[2], "2026-01-01T00:00:02Z"),
    ] {
        conn.execute("UPDATE task SET done_at = ?1 WHERE id = ?2", (at, id.0))
            .unwrap();
    }
    assert_eq!(titles(&conn), ["b", "d", "c", "a"]);
}

#[test]
fn set_done_is_idempotent_and_keeps_done_at() {
    let conn = db::test_conn();
    let id = mk(&conn, "a").unwrap();
    conn.execute("UPDATE task SET done_at = '2026-01-01T00:00:00Z'", [])
        .unwrap();
    set_done(&conn, id, true, d("2026-06-15")).unwrap();
    assert_eq!(done_at(&conn, id).as_deref(), Some("2026-01-01T00:00:00Z"));
}

#[test]
fn set_done_sets_and_clears() {
    let conn = db::test_conn();
    let id = mk(&conn, "a").unwrap();
    set_done(&conn, id, true, d("2026-06-15")).unwrap();
    assert!(done_at(&conn, id).is_some());
    assert!(ls(&conn)[0].done);
    set_done(&conn, id, false, d("2026-06-15")).unwrap();
    assert!(done_at(&conn, id).is_none());
    assert!(!ls(&conn)[0].done);
}

#[test]
fn update_changes_title_and_note_and_trims() {
    let conn = db::test_conn();
    let id = mk(&conn, "a").unwrap();
    update(
        &conn,
        id,
        "  new title 
",
        "  line 1

line 3 
	 ",
        Priority {
            urgent: true,
            important: false,
        },
        "",
        "",
        None,
    )
    .unwrap();
    let t = &ls(&conn)[0];
    assert_eq!(t.title, "new title");
    assert_eq!(
        t.priority,
        Priority {
            urgent: true,
            important: false
        }
    );
    assert_eq!(
        t.note,
        "  line 1

line 3"
    );
    update(
        &conn,
        id,
        "new title",
        "",
        Priority::default(),
        "",
        "",
        None,
    )
    .unwrap();
    assert_eq!(ls(&conn)[0].note, "");
}

#[test]
fn update_with_empty_title_changes_nothing() {
    let conn = db::test_conn();
    let id = mk(&conn, "a").unwrap();
    update(&conn, id, "a", "keep", Priority::default(), "", "", None).unwrap();
    assert!(matches!(
        update(
            &conn,
            id,
            " 
",
            "other",
            Priority::default(),
            "",
            "",
            None
        ),
        Err(TaskError::EmptyTitle)
    ));
    let t = &ls(&conn)[0];
    assert_eq!((t.title.as_str(), t.note.as_str()), ("a", "keep"));
}

fn t(s: &str) -> NaiveTime {
    NaiveTime::parse_from_str(s, "%H:%M").unwrap()
}

fn times(conn: &Connection, id: TaskId) -> (Option<NaiveTime>, Option<NaiveTime>) {
    let t = ls(conn).into_iter().find(|x| x.id.0 == id.0).unwrap();
    (t.start, t.end)
}

#[test]
fn parse_time_accepts_h_mm_and_hh_mm() {
    assert_eq!(parse_time("9:05").unwrap(), Some(t("09:05")));
    assert_eq!(parse_time(" 09:05 ").unwrap(), Some(t("09:05")));
    assert_eq!(parse_time("23:59").unwrap(), Some(t("23:59")));
    assert_eq!(parse_time("0:00").unwrap(), Some(t("00:00")));
    assert_eq!(parse_time("").unwrap(), None);
    assert_eq!(parse_time("  ").unwrap(), None);
    for bad in [
        "24:00", "12:60", "9", "9:5", "009:00", "12:345", "ab:cd", "+9:00", "-1:00", "9.30", ":30",
        "12:", "12:3x",
    ] {
        assert!(
            matches!(parse_time(bad), Err(TaskError::BadTime(_))),
            "{bad}"
        );
    }
}

#[test]
fn time_rules_in_set_time() {
    let conn = db::test_conn();
    let id = mk(&conn, "a").unwrap();
    // A time needs a date.
    assert!(matches!(
        set_time(&conn, id, Some(t("09:00")), None),
        Err(TaskError::TimeWithoutDate)
    ));
    assert!(matches!(
        set_time(&conn, TaskId(42), Some(t("09:00")), None),
        Err(TaskError::NotFound)
    ));
    // Clearing is always fine.
    set_time(&conn, id, None, None).unwrap();
    set_due_date(&conn, id, Some(d("2026-06-15"))).unwrap();
    // The end needs a start and must be strictly later.
    assert!(matches!(
        set_time(&conn, id, None, Some(t("10:00"))),
        Err(TaskError::EndWithoutStart)
    ));
    for end in ["09:00", "08:59"] {
        assert!(matches!(
            set_time(&conn, id, Some(t("09:00")), Some(t(end))),
            Err(TaskError::EndNotAfterStart)
        ));
    }
    assert_eq!(times(&conn, id), (None, None));
    set_time(&conn, id, Some(t("14:15")), Some(t("16:00"))).unwrap();
    assert_eq!(times(&conn, id), (Some(t("14:15")), Some(t("16:00"))));
    let raw: (String, String) = conn
        .query_row("SELECT due_time, end_time FROM task", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(raw, ("14:15".into(), "16:00".into()));
    set_time(&conn, id, Some(t("14:15")), None).unwrap();
    assert_eq!(times(&conn, id), (Some(t("14:15")), None));
}

#[test]
fn due_date_changes_and_times() {
    let conn = db::test_conn();
    let id = mk(&conn, "a").unwrap();
    set_due_date(&conn, id, Some(d("2026-06-15"))).unwrap();
    set_time(&conn, id, Some(t("09:00")), Some(t("10:30"))).unwrap();
    // Rescheduling keeps the times; removing the date clears both.
    set_due_date(&conn, id, Some(d("2026-06-20"))).unwrap();
    assert_eq!(times(&conn, id), (Some(t("09:00")), Some(t("10:30"))));
    set_due_date(&conn, id, None).unwrap();
    assert_eq!(times(&conn, id), (None, None));
    set_due_date(&conn, id, Some(d("2026-06-21"))).unwrap();
    assert_eq!(times(&conn, id), (None, None));
}

#[test]
fn update_saves_times_atomically() {
    let conn = db::test_conn();
    let id = mk(&conn, "a").unwrap();
    let go = |title: &str, s: &str, e: &str| {
        update(&conn, id, title, "n", Priority::default(), s, e, None)
    };
    // Undated: a start is refused and nothing else changes.
    assert!(matches!(
        go("b", "9:00", ""),
        Err(TaskError::TimeWithoutDate)
    ));
    assert_eq!(ls(&conn)[0].title, "a");
    go("b", "", "").unwrap();
    set_due_date(&conn, id, Some(d("2026-06-15"))).unwrap();
    for (s, e) in [
        ("xx", ""),
        ("", "10:00"),
        ("10:00", "9:00"),
        ("10:00", "10:00"),
    ] {
        assert!(go("c", s, e).is_err(), "{s} {e}");
        assert_eq!(ls(&conn)[0].title, "b");
    }
    go("c", "9:00", "10:30").unwrap();
    let task = &ls(&conn)[0];
    assert_eq!(task.title, "c");
    assert_eq!(task.time_label(), "09:00\u{2013}10:30");
    assert_eq!(task.timed_title(), "09:00 c");
    go("c", "", "").unwrap();
    assert_eq!(times(&conn, id), (None, None));
    assert_eq!(ls(&conn)[0].timed_title(), "c");
}

#[test]
fn update_and_delete_unknown_id_are_not_found() {
    let conn = db::test_conn();
    assert!(matches!(
        update(
            &conn,
            TaskId(42),
            "a",
            "",
            Priority::default(),
            "",
            "",
            None
        ),
        Err(TaskError::NotFound)
    ));
    assert!(matches!(
        delete(&conn, TaskId(42)),
        Err(TaskError::NotFound)
    ));
}

#[test]
fn delete_removes_the_task_and_its_count() {
    let conn = db::test_conn();
    let a = mk(&conn, "a").unwrap();
    mk(&conn, "b").unwrap();
    assert_eq!(counts(&conn, at(NOW)).unwrap().all, 2);
    delete(&conn, a).unwrap();
    assert_eq!(titles(&conn), ["b"]);
    assert_eq!(counts(&conn, at(NOW)).unwrap().all, 1);
    assert!(matches!(delete(&conn, a), Err(TaskError::NotFound)));
}

#[test]
fn unknown_id_is_not_found() {
    let conn = db::test_conn();
    assert!(matches!(
        set_done(&conn, TaskId(42), true, d("2026-06-15")),
        Err(TaskError::NotFound)
    ));
    assert!(matches!(
        set_done(&conn, TaskId(42), false, d("2026-06-15")),
        Err(TaskError::NotFound)
    ));
}

#[test]
fn create_sets_due_by_view_and_shows_up_there() {
    let conn = db::test_conn();
    let today = d("2026-06-15");
    for (title, view) in [
        ("t", View::Today),
        ("s", View::Scheduled),
        ("i", View::Inbox),
        ("a", View::All),
    ] {
        create(&conn, title, view, today).unwrap();
        assert!(view_titles(&conn, view, NOW).contains(&title.to_string()));
    }
    let dues: Vec<_> = ls(&conn).into_iter().map(|t| t.due).collect();
    assert_eq!(dues, [Some(today), Some(d("2026-06-16")), None, None]);
}

#[test]
fn week_puts_each_task_on_its_due_day() {
    let conn = db::test_conn();
    for (title, due, done) in [
        ("mon", Some("2026-11-02"), false),
        ("sun", Some("2026-11-08"), false),
        ("thu-done", Some("2026-11-05"), true),
        ("thu", Some("2026-11-05"), false),
        ("before", Some("2026-11-01"), false),
        ("after", Some("2026-11-09"), false),
        ("none", None, false),
    ] {
        let id = mk(&conn, title).unwrap();
        set_due_date(&conn, id, due.map(d)).unwrap();
        set_done(&conn, id, done, d("2026-06-15")).unwrap();
    }
    let w = week(&conn, d("2026-11-02")).unwrap();
    let titles: Vec<Vec<&str>> = w
        .iter()
        .map(|day| day.iter().map(|t| t.title.as_str()).collect())
        .collect();
    let empty: Vec<&str> = vec![];
    assert_eq!(
        titles,
        [
            vec!["mon"],
            empty.clone(),
            empty.clone(),
            // Pending before done.
            vec!["thu", "thu-done"],
            empty.clone(),
            empty.clone(),
            vec!["sun"],
        ]
    );
}

fn kanban_titles(conn: &Connection) -> Vec<Vec<String>> {
    let now = DateTime::parse_from_rfc3339("2026-06-15T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    kanban(conn, now)
        .unwrap()
        .into_iter()
        .map(|c| c.into_iter().map(|t| t.title).collect())
        .collect()
}

#[test]
fn kanban_moves_tasks_between_columns() {
    let conn = db::test_conn();
    let a = mk(&conn, "a").unwrap();
    let b = mk(&conn, "b").unwrap();
    let c = mk(&conn, "c").unwrap();
    set_due_date(&conn, b, Some(d("2026-06-20"))).unwrap();
    // Dated before undated in the icebox.
    assert_eq!(kanban_titles(&conn), [vec!["b", "a", "c"], vec![], vec![]]);
    set_stage(&conn, a, Stage::InProgress, d("2026-06-15")).unwrap();
    // Finished within the last 7 days.
    conn.execute(
        "UPDATE task SET done_at = '2026-06-10T00:00:00.000Z' WHERE id = ?1",
        [c.0],
    )
    .unwrap();
    assert_eq!(kanban_titles(&conn), [vec!["b"], vec!["a"], vec!["c"]]);
    // Done then unchecked: back to the column it came from.
    set_stage(&conn, a, Stage::Done, d("2026-06-15")).unwrap();
    set_done(&conn, a, false, d("2026-06-15")).unwrap();
    assert_eq!(kanban_titles(&conn)[1], ["a"]);
    set_stage(&conn, a, Stage::Icebox, d("2026-06-15")).unwrap();
    assert_eq!(kanban_titles(&conn)[0], ["b", "a"]);
    assert!(matches!(
        set_stage(&conn, TaskId(42), Stage::InProgress, d("2026-06-15")),
        Err(TaskError::NotFound)
    ));
}

#[test]
fn kanban_done_column_keeps_only_the_last_7_days() {
    let conn = db::test_conn();
    for (title, done_at) in [
        ("old", "2026-06-08T11:59:59.999Z"),
        ("edge", "2026-06-08T12:00:00.000Z"),
    ] {
        let id = mk(&conn, title).unwrap();
        conn.execute(
            "UPDATE task SET done_at = ?1 WHERE id = ?2",
            (done_at, id.0),
        )
        .unwrap();
    }
    assert_eq!(kanban_titles(&conn)[2], ["edge"]);
}

#[test]
fn quadrant_indexes_match_the_quadrants() {
    for (i, q) in Priority::QUADRANTS.iter().enumerate() {
        assert_eq!(q.quadrant(), i, "{q:?}");
    }
}

#[test]
fn eisenhower_sorts_pending_tasks_into_quadrants() {
    let conn = db::test_conn();
    let p = |urgent, important| Priority { urgent, important };
    for (title, priority, done) in [
        ("do", p(true, true), false),
        ("plan", p(false, true), false),
        ("delegate", p(true, false), false),
        ("drop", p(false, false), false),
        ("done", p(true, true), true),
    ] {
        let id = mk(&conn, title).unwrap();
        set_priority(&conn, id, priority).unwrap();
        set_done(&conn, id, done, d("2026-06-15")).unwrap();
    }
    let titles = |conn: &Connection| -> Vec<Vec<String>> {
        eisenhower(conn)
            .unwrap()
            .into_iter()
            .map(|q| q.into_iter().map(|t| t.title).collect())
            .collect()
    };
    // Done tasks stay out.
    assert_eq!(
        titles(&conn),
        [vec!["do"], vec!["plan"], vec!["delegate"], vec!["drop"]]
    );
    let drop_id = ls(&conn).iter().find(|t| t.title == "drop").unwrap().id;
    set_priority(&conn, drop_id, p(true, true)).unwrap();
    assert_eq!(titles(&conn)[0], ["do", "drop"]);
    assert!(matches!(
        set_priority(&conn, TaskId(42), p(true, true)),
        Err(TaskError::NotFound)
    ));
}

#[test]
fn overdue_takes_no_new_tasks() {
    let conn = db::test_conn();
    let r = create(&conn, "a", View::Overdue, d("2026-06-15"));
    assert!(matches!(r, Err(TaskError::CannotCreateHere)));
    assert!(ls(&conn).is_empty());
}

#[test]
fn smart_views_split_tasks_by_due_date_and_list() {
    let conn = db::test_conn();
    let work = lists::create(&conn, "Work").unwrap();
    for (title, due, list) in [
        ("today", Some("2026-06-15"), None),
        ("future", Some("2026-06-16"), None),
        ("none", None, None),
        ("listed", None, Some(work)),
        ("late", Some("2026-06-10"), None),
        ("later", Some("2026-06-12"), Some(work)),
    ] {
        let id = mk(&conn, title).unwrap();
        set_due_date(&conn, id, due.map(d)).unwrap();
        set_list(&conn, id, list).unwrap();
    }
    let v = |view| view_titles(&conn, view, NOW);
    assert_eq!(v(View::Today), ["today"]);
    assert_eq!(v(View::Scheduled), ["future"]);
    // A task with a list is not in the Inbox, even without a date.
    assert_eq!(v(View::Inbox), ["none"]);
    assert_eq!(v(View::Overdue), ["late", "later"]);
}

#[test]
fn counts_match_the_pending_tasks_each_view_lists() {
    let conn = db::test_conn();
    let work = lists::create(&conn, "Work").unwrap();
    for (due, list, done) in [
        (Some("2026-06-15"), None, false),
        (Some("2026-06-15"), None, true),
        (Some("2026-06-20"), Some(work), false),
        (None, None, false),
        (None, None, true),
        (None, Some(work), false),
        (Some("2026-06-01"), None, false),
        (Some("2026-06-01"), Some(work), true),
    ] {
        let id = mk(&conn, "t").unwrap();
        set_due_date(&conn, id, due.map(d)).unwrap();
        set_list(&conn, id, list).unwrap();
        set_done(&conn, id, done, d("2026-06-15")).unwrap();
    }
    let c = counts(&conn, at(NOW)).unwrap();
    for view in [
        View::Today,
        View::Scheduled,
        View::Inbox,
        View::Overdue,
        View::All,
        View::List(work),
    ] {
        let tasks = list(&conn, view, at(NOW)).unwrap();
        let pending = tasks.iter().filter(|t| !t.done).count();
        assert_eq!(pending, c.for_view(view) as usize, "{view:?}");
        // Pending first, so the done ones start right after them.
        assert_eq!(first_done(&tasks), pending, "{view:?}");
        assert!(tasks[first_done(&tasks)..].iter().all(|t| t.done));
    }
}

#[test]
fn today_lists_done_today_after_pending() {
    let conn = db::test_conn();
    for (title, due, done) in [
        ("done-today", "2026-06-15", Some("2026-06-15T08:00:00.000Z")),
        ("done-later", "2026-06-14", Some("2026-06-15T09:00:00.000Z")),
        (
            "done-yesterday",
            "2026-06-15",
            Some("2026-06-14T23:59:59.999Z"),
        ),
        (
            "done-future-due",
            "2026-06-20",
            Some("2026-06-15T10:00:00.000Z"),
        ),
        ("pending", "2026-06-15", None),
    ] {
        let id = mk(&conn, title).unwrap();
        conn.execute(
            "UPDATE task SET due_date = ?1, done_at = ?2 WHERE id = ?3",
            (due, done, id.0),
        )
        .unwrap();
    }
    assert_eq!(today_titles(&conn, NOW), ["pending", "done-today"]);
    // Overdue keeps what was finished today, so checking a task does not hide it.
    assert_eq!(view_titles(&conn, View::Overdue, NOW), ["done-later"]);
}

#[test]
fn today_midnight_uses_the_offset_of_now() {
    let conn = db::test_conn();
    for (title, at) in [
        ("before", "2026-06-15T02:59:59.999Z"),
        ("after", "2026-06-15T03:00:00.000Z"),
    ] {
        let id = mk(&conn, title).unwrap();
        conn.execute(
            "UPDATE task SET due_date = '2026-06-15', done_at = ?1 WHERE id = ?2",
            (at, id.0),
        )
        .unwrap();
    }
    // 22:00 at UTC-3 on the 15th: local midnight is 03:00Z.
    assert_eq!(today_titles(&conn, "2026-06-15T22:00:00-03:00"), ["after"]);
}

#[test]
fn set_due_date_sets_clears_and_not_found() {
    let conn = db::test_conn();
    let id = mk(&conn, "a").unwrap();
    set_due_date(&conn, id, Some(d("2026-07-01"))).unwrap();
    assert_eq!(ls(&conn)[0].due, Some(d("2026-07-01")));
    set_due_date(&conn, id, None).unwrap();
    assert_eq!(ls(&conn)[0].due, None);
    assert!(matches!(
        set_due_date(&conn, TaskId(42), None),
        Err(TaskError::NotFound)
    ));
}

#[test]
fn invalid_date_is_rejected_by_the_database() {
    let conn = db::test_conn();
    mk(&conn, "a").unwrap();
    assert!(
        conn.execute("UPDATE task SET due_date = '2026-13-40'", [])
            .is_err()
    );
}

fn list_of(conn: &Connection, id: TaskId) -> Option<(ListId, String)> {
    ls(conn).into_iter().find(|t| t.id == id).unwrap().list
}

#[test]
fn create_in_list_view_assigns_the_list_without_due() {
    let conn = db::test_conn();
    let work = lists::create(&conn, "Work").unwrap();
    let id = create(&conn, "a", View::List(work), d("2026-06-15")).unwrap();
    let t = ls(&conn).remove(0);
    assert_eq!(t.id, id);
    assert_eq!(t.due, None);
    assert_eq!(t.list, Some((work, "Work".to_string())));
}

#[test]
fn create_in_unknown_list_fails_cleanly() {
    let conn = db::test_conn();
    let r = create(&conn, "a", View::List(ListId(42)), d("2026-06-15"));
    assert!(matches!(r, Err(TaskError::ListNotFound)));
    assert!(ls(&conn).is_empty());
}

#[test]
fn list_view_shows_only_that_list_in_all_order() {
    let conn = db::test_conn();
    let work = lists::create(&conn, "Work").unwrap();
    let home = lists::create(&conn, "Home").unwrap();
    for (title, list) in [
        ("w1", Some(work)),
        ("h1", Some(home)),
        ("n1", None),
        ("w2", Some(work)),
    ] {
        let id = mk(&conn, title).unwrap();
        set_list(&conn, id, list).unwrap();
    }
    conn.execute(
        "UPDATE task SET done_at = '2026-01-01T00:00:00Z' WHERE title = 'w1'",
        [],
    )
    .unwrap();
    let v = list(&conn, View::List(work), at(NOW)).unwrap();
    let titles: Vec<_> = v.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(titles, ["w2", "w1"]);
}

#[test]
fn set_list_moves_clears_and_reports_errors() {
    let conn = db::test_conn();
    let work = lists::create(&conn, "Work").unwrap();
    let id = mk(&conn, "a").unwrap();
    assert_eq!(list_of(&conn, id), None);
    set_list(&conn, id, Some(work)).unwrap();
    assert_eq!(list_of(&conn, id), Some((work, "Work".to_string())));
    set_list(&conn, id, None).unwrap();
    assert_eq!(list_of(&conn, id), None);
    assert!(matches!(
        set_list(&conn, TaskId(42), Some(work)),
        Err(TaskError::NotFound)
    ));
    assert!(matches!(
        set_list(&conn, id, Some(ListId(42))),
        Err(TaskError::ListNotFound)
    ));
    assert_eq!(list_of(&conn, id), None);
}

#[test]
fn counts_pending_per_view_and_list() {
    let conn = db::test_conn();
    let work = lists::create(&conn, "Work").unwrap();
    let empty = lists::create(&conn, "Empty").unwrap();
    // (list, due, done)
    for (list, due, done) in [
        (Some(work), Some("2026-06-15"), false),
        (Some(work), Some("2026-06-10"), false),
        (Some(work), Some("2026-06-15"), true),
        (None, Some("2026-06-16"), false),
        (None, None, false),
        (None, Some("2026-06-15"), false),
    ] {
        let id = mk(&conn, "t").unwrap();
        set_list(&conn, id, list).unwrap();
        set_due_date(&conn, id, due.map(d)).unwrap();
        if done {
            set_done(&conn, id, true, d("2026-06-15")).unwrap();
        }
    }
    let c = counts(&conn, at(NOW)).unwrap();
    assert_eq!(
        (c.today, c.scheduled, c.inbox, c.overdue, c.all),
        (2, 1, 1, 1, 5)
    );
    assert_eq!(c.for_list(work), 2);
    assert_eq!(c.for_list(empty), 0);
    assert_eq!(c.per_list, [(work, 2)]);
}

#[test]
fn due_label_cases() {
    let today = d("2026-06-15");
    assert_eq!(due_label(&task(None, false), today), None);
    let l = |due, done| due_label(&task(Some(due), done), today);
    assert_eq!(l("2026-06-14", false), Some(DueLabel::Overdue));
    assert_eq!(l("2026-06-14", true), Some(DueLabel::On(d("2026-06-14"))));
    assert_eq!(l("2026-06-15", false), Some(DueLabel::Today));
    assert_eq!(l("2026-06-15", true), Some(DueLabel::Today));
    assert_eq!(l("2026-06-16", false), Some(DueLabel::Tomorrow));
    assert_eq!(l("2026-06-17", false), Some(DueLabel::On(d("2026-06-17"))));
}

// --- repeat ---

/// Every task, oldest first (the list puts pending ones before done ones).
fn byid(conn: &Connection) -> Vec<Task> {
    let mut all = ls(conn);
    all.sort_by_key(|t| t.id.0);
    all
}

const TODAY: &str = "2026-06-15";

/// A task due on `due`, repeating every 2 months, with every copied field set.
fn repeating(conn: &Connection, due: &str) -> TaskId {
    let id = create_on(conn, "Pay rent", d(due)).unwrap();
    let list = lists::create(conn, "Home").unwrap();
    set_list(conn, id, Some(list)).unwrap();
    update(
        conn,
        id,
        "Pay rent",
        "bank transfer",
        Priority {
            urgent: true,
            important: true,
        },
        "09:00",
        "10:30",
        Some((Frequency::Monthly, "2")),
    )
    .unwrap();
    id
}

fn rules(conn: &Connection) -> Vec<(i64, String, u32, u32)> {
    conn.prepare("SELECT task_id, frequency, interval, anchor_day FROM task_recurrence")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn completing_creates_the_next_task_with_every_field_and_moves_the_rule() {
    let conn = db::test_conn();
    let id = repeating(&conn, "2026-06-20");
    set_stage(&conn, id, Stage::InProgress, d(TODAY)).unwrap();
    set_done(&conn, id, true, d(TODAY)).unwrap();
    let tasks = byid(&conn);
    assert_eq!(tasks.len(), 2);
    let (old, next) = (&tasks[0], &tasks[1]);
    assert_eq!((old.id, old.done, old.repeat), (id, true, None));
    assert_eq!(next.due, Some(d("2026-08-20")));
    assert!(!next.done);
    assert_eq!(next.title, "Pay rent");
    assert_eq!(next.note, "bank transfer");
    assert_eq!(next.list.as_ref().map(|l| l.1.as_str()), Some("Home"));
    assert_eq!(
        next.priority,
        Priority {
            urgent: true,
            important: true
        }
    );
    assert_eq!(next.time_label(), "09:00\u{2013}10:30");
    assert_eq!(next.repeat, Some((Frequency::Monthly, 2)));
    assert_eq!(rules(&conn), [(next.id.0, "monthly".into(), 2, 20)]);
    let in_progress: bool = conn
        .query_row(
            "SELECT in_progress FROM task WHERE id = ?1",
            [next.id.0],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!in_progress);
}

#[test]
fn anchor_day_survives_a_short_month() {
    let conn = db::test_conn();
    let id = create_on(&conn, "x", d("2027-01-31")).unwrap();
    set_recurrence(&conn, id, Some((Frequency::Monthly, ""))).unwrap();
    set_done(&conn, id, true, d("2027-01-31")).unwrap();
    let second = byid(&conn)[1].id;
    assert_eq!(byid(&conn)[1].due, Some(d("2027-02-28")));
    set_done(&conn, second, true, d("2027-02-28")).unwrap();
    assert_eq!(byid(&conn)[2].due, Some(d("2027-03-31")));
}

#[test]
fn unchecking_and_completing_again_never_duplicates() {
    let conn = db::test_conn();
    let id = repeating(&conn, "2026-06-20");
    set_done(&conn, id, true, d(TODAY)).unwrap();
    set_done(&conn, id, false, d(TODAY)).unwrap();
    assert_eq!(byid(&conn).len(), 2);
    assert!(!byid(&conn)[0].done);
    set_done(&conn, id, true, d(TODAY)).unwrap();
    set_done(&conn, id, true, d(TODAY)).unwrap();
    assert_eq!(byid(&conn).len(), 2);
    assert_eq!(rules(&conn).len(), 1);
}

#[test]
fn a_very_late_task_does_not_come_back_overdue() {
    let conn = db::test_conn();
    let id = create_on(&conn, "x", d("2026-01-10")).unwrap();
    set_recurrence(&conn, id, Some((Frequency::Weekly, "1"))).unwrap();
    set_done(&conn, id, true, d(TODAY)).unwrap();
    // 10 Jan + 7n: the first one after 15 Jun 2026 (a Monday) is Saturday 20 Jun.
    assert_eq!(byid(&conn)[1].due, Some(d("2026-06-20")));
}

#[test]
fn a_task_without_a_rule_creates_nothing() {
    let conn = db::test_conn();
    let id = create_on(&conn, "x", d("2026-06-10")).unwrap();
    set_done(&conn, id, true, d(TODAY)).unwrap();
    assert_eq!(byid(&conn).len(), 1);
}

#[test]
fn the_kanban_done_column_makes_the_next_task_too() {
    let conn = db::test_conn();
    let id = repeating(&conn, "2026-06-20");
    set_stage(&conn, id, Stage::Done, d(TODAY)).unwrap();
    assert_eq!(byid(&conn).len(), 2);
    assert_eq!(byid(&conn)[1].due, Some(d("2026-08-20")));
}

#[test]
fn deleting_a_task_deletes_its_rule() {
    let conn = db::test_conn();
    let id = repeating(&conn, "2026-06-20");
    delete(&conn, id).unwrap();
    assert!(rules(&conn).is_empty());
}

#[test]
fn a_rule_needs_a_date_and_a_valid_interval() {
    let conn = db::test_conn();
    let id = mk(&conn, "x").unwrap();
    assert!(matches!(
        set_recurrence(&conn, id, Some((Frequency::Daily, "1"))),
        Err(TaskError::RecurrenceWithoutDate)
    ));
    assert!(matches!(
        update(
            &conn,
            id,
            "x",
            "",
            Priority::default(),
            "",
            "",
            Some((Frequency::Daily, ""))
        ),
        Err(TaskError::RecurrenceWithoutDate)
    ));
    assert!(matches!(
        set_recurrence(&conn, TaskId(99), Some((Frequency::Daily, "1"))),
        Err(TaskError::NotFound)
    ));
    set_due_date(&conn, id, Some(d("2026-06-20"))).unwrap();
    for bad in ["0", "-1", "x", "1.5", "1000"] {
        assert!(
            matches!(
                set_recurrence(&conn, id, Some((Frequency::Daily, bad))),
                Err(TaskError::BadInterval(_))
            ),
            "{bad}"
        );
    }
    assert!(rules(&conn).is_empty());
    set_recurrence(&conn, id, Some((Frequency::Daily, " 3 "))).unwrap();
    assert_eq!(byid(&conn)[0].repeat, Some((Frequency::Daily, 3)));
    set_recurrence(&conn, id, None).unwrap();
    assert_eq!(byid(&conn)[0].repeat, None);
}

#[test]
fn clearing_the_date_clears_the_rule_and_a_new_date_moves_the_anchor() {
    let conn = db::test_conn();
    let id = create_on(&conn, "x", d("2026-06-30")).unwrap();
    set_recurrence(&conn, id, Some((Frequency::Monthly, "1"))).unwrap();
    set_due_date(&conn, id, Some(d("2026-07-12"))).unwrap();
    assert_eq!(rules(&conn), [(id.0, "monthly".into(), 1, 12)]);
    set_due_date(&conn, id, None).unwrap();
    assert!(rules(&conn).is_empty());
}

#[test]
fn a_failed_save_or_completion_rolls_everything_back() {
    let conn = db::test_conn();
    let id = create_on(&conn, "x", d("2026-06-20")).unwrap();
    // The new title must not stay when the interval is bad.
    assert!(matches!(
        update(
            &conn,
            id,
            "y",
            "",
            Priority::default(),
            "",
            "",
            Some((Frequency::Daily, "0"))
        ),
        Err(TaskError::BadInterval(_))
    ));
    assert_eq!(byid(&conn)[0].title, "x");
    // A rule that cannot advance fails after the task was marked done: nothing stays.
    conn.execute(
        "UPDATE task SET due_date = '9999-12-31' WHERE id = ?1",
        [id.0],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO task_recurrence VALUES (?1, 'daily', 1, 31)",
        [id.0],
    )
    .unwrap();
    assert!(set_done(&conn, id, true, d(TODAY)).is_err());
    assert!(!byid(&conn)[0].done);
    assert_eq!(byid(&conn).len(), 1);
}

#[test]
fn migration_keeps_existing_tasks() {
    let dir = std::env::temp_dir().join(format!("op-tasks-{}-rec-migrate", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("old.db");
    {
        let conn = Connection::open(&path).unwrap();
        for sql in [
            include_str!("../../migrations/0001_baseline.sql"),
            include_str!("../../migrations/0002_tasks.sql"),
            include_str!("../../migrations/0003_task_due_date.sql"),
            include_str!("../../migrations/0004_lists.sql"),
            include_str!("../../migrations/0005_task_note.sql"),
            include_str!("../../migrations/0006_task_in_progress.sql"),
            include_str!("../../migrations/0007_task_priority.sql"),
            include_str!("../../migrations/0008_habits.sql"),
            include_str!("../../migrations/0009_task_time.sql"),
        ] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 9).unwrap();
        conn.execute(
            "INSERT INTO task (title, due_date) VALUES ('old', '2026-06-20')",
            [],
        )
        .unwrap();
    }
    let conn = db::open(&path).unwrap();
    let t = &byid(&conn)[0];
    assert_eq!(
        (t.title.as_str(), t.due, t.repeat),
        ("old", Some(d("2026-06-20")), None)
    );
    set_recurrence(&conn, t.id, Some((Frequency::Yearly, "1"))).unwrap();
    drop(conn);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn topics_are_not_inbox_but_dated_ones_show_in_today() {
    let conn = db::test_conn();
    mk(&conn, "plain").unwrap();
    let math = crate::study::create_subject(&conn, "Math").unwrap();
    crate::study::create_topic(&conn, math, "undated", None).unwrap();
    crate::study::create_topic(&conn, math, "dated", Some(d("2026-06-15"))).unwrap();
    assert_eq!(view_titles(&conn, View::Inbox, NOW), ["plain"]);
    assert_eq!(counts(&conn, at(NOW)).unwrap().inbox, 1);
    assert_eq!(view_titles(&conn, View::Today, NOW), ["dated"]);
}
