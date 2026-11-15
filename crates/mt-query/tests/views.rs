//! View definitions: kinds, layout, colour rules and who may change a view.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_core::condition::{Condition, Operand, Operator, Period, PeriodUnit};
use mt_core::id::{TableId, ViewId};
use mt_core::value::Value;
use mt_query::view::{MAX_COLOUR_RULES, MAX_COLUMN_WIDTH, MAX_VIEW_NAME_CHARS, MIN_COLUMN_WIDTH};
use mt_query::{
    ColourRule, ColourTarget, FieldSetting, GroupKey, QueryError, RowHeight, SortKey, View,
    ViewAccess, ViewKind,
};

fn view() -> View {
    View::grid(
        ViewId::from_u128(1),
        TableId::from_u128(1),
        "All deals",
        person(1),
    )
}

fn setting(field: u128, visible: bool, width: Option<u16>) -> FieldSetting {
    FieldSetting {
        field: fid(field),
        visible,
        width,
    }
}

fn overdue() -> Condition {
    Condition::All(vec![
        Condition::test(
            fid(CLOSE),
            Operator::Lt,
            Operand::Period(Period::this(PeriodUnit::Day)),
        ),
        Condition::compare(fid(DONE), Operator::Ne, Value::Bool(true)),
    ])
}

fn reason(view: &View) -> String {
    view.check(&type_of).unwrap_err().to_string()
}

#[test]
fn std_grid_view_with_layout_query_and_colours() {
    let mut v = view();
    v.fields = vec![
        setting(TITLE, true, Some(280)),
        setting(STAGE, true, None),
        setting(NOTES, false, None),
        setting(AMOUNT, true, Some(120)),
    ];
    v.frozen = 1;
    v.row_height = RowHeight::Medium;
    v.query.sort = vec![SortKey::desc(fid(AMOUNT))];
    v.query.group = vec![GroupKey::by(fid(STAGE))];
    v.colours = vec![
        ColourRule {
            when: overdue(),
            target: ColourTarget::Record,
            colour: "red".into(),
        },
        ColourRule {
            when: Condition::compare(fid(DONE), Operator::Eq, Value::Bool(true)),
            target: ColourTarget::Record,
            colour: "green".into(),
        },
        ColourRule {
            when: Condition::is_empty(fid(AMOUNT)),
            target: ColourTarget::Field { field: fid(AMOUNT) },
            colour: "amber-2".into(),
        },
    ];
    v.check(&type_of).unwrap();
    assert_eq!(
        v.visible_fields().collect::<Vec<_>>(),
        vec![fid(TITLE), fid(STAGE), fid(AMOUNT)]
    );
    let json = serde_json::to_string(&v).unwrap();
    assert_eq!(serde_json::from_str::<View>(&json).unwrap(), v);

    // Colours: the first matching rule per target wins.
    let rows = deals();
    let colour = |id: u128, target: ColourTarget| {
        let row = rows.iter().find(|r| r.id == rec(id)).unwrap();
        v.colour_of(target, &|f| row.cells.get(&f), &ctx())
            .map(str::to_owned)
    };
    // Today is 2026-05-13. Deal 5 closed in April and is done: the first rule does not match
    // (it is done), the second does.
    assert_eq!(colour(5, ColourTarget::Record), Some("green".to_owned()));
    // Deal 2 closes on 05-20: not overdue, not done.
    assert_eq!(colour(2, ColourTarget::Record), None);
    // Deal 6 has no amount.
    assert_eq!(
        colour(6, ColourTarget::Field { field: fid(AMOUNT) }),
        Some("amber-2".to_owned())
    );
    assert_eq!(colour(6, ColourTarget::Field { field: fid(TITLE) }), None);
    assert_eq!(colour(1, ColourTarget::Field { field: fid(AMOUNT) }), None);
    // An overdue, open deal is red even though a later rule might also match.
    let mut late = rows[1].clone();
    late.cells.insert(fid(CLOSE), date(2026, 5, 1));
    assert_eq!(
        v.colour_of(ColourTarget::Record, &|f| late.cells.get(&f), &ctx()),
        Some("red")
    );
}

#[test]
fn std_every_kind_of_view() {
    let kinds = [
        ViewKind::Grid,
        ViewKind::List,
        ViewKind::Kanban {
            stack_by: fid(STAGE),
        },
        ViewKind::Kanban {
            stack_by: fid(DONE),
        },
        ViewKind::Kanban {
            stack_by: fid(OWNERS),
        },
        ViewKind::Calendar {
            start: fid(CLOSE),
            end: None,
        },
        ViewKind::Calendar {
            start: fid(CREATED),
            end: Some(fid(CLOSE)),
        },
        ViewKind::Timeline {
            start: fid(CREATED),
            end: fid(CLOSE),
        },
        ViewKind::Gallery { cover: None },
        ViewKind::Gallery {
            cover: Some(fid(FILES)),
        },
    ];
    for kind in kinds {
        let mut v = view();
        v.kind = kind;
        v.check(&type_of).unwrap();
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(serde_json::from_str::<View>(&json).unwrap(), v, "{json}");
    }
    // A view stored with only the required keys loads with defaults.
    let minimal = r#"{"id":"viw_00000000000000000000000001","table":"tbl_00000000000000000000000001",
        "name":"V","kind":{"kind":"grid"},"owner":"prn_00000000000000000000000001"}"#;
    let v: View = serde_json::from_str(minimal).unwrap();
    assert_eq!(
        (v.access, v.row_height, v.frozen),
        (ViewAccess::Shared, RowHeight::Short, 0)
    );
    let field: FieldSetting =
        serde_json::from_str(r#"{"field":"fld_00000000000000000000000001"}"#).unwrap();
    assert!(field.visible && field.width.is_none());
}

#[test]
fn invalid_views() {
    let with = |edit: &dyn Fn(&mut View)| {
        let mut v = view();
        edit(&mut v);
        reason(&v)
    };
    assert!(with(&|v| v.name = "  ".into()).contains("the name must be"));
    assert!(with(&|v| v.name = "x".repeat(MAX_VIEW_NAME_CHARS + 1)).contains("the name must be"));
    assert!(with(&|v| v.name = "a\tb".into()).contains("the name must be"));
    // The query is checked too.
    let mut v = view();
    v.query.sort = vec![SortKey::asc(fid(99))];
    assert!(matches!(
        v.check(&type_of),
        Err(QueryError::Invalid { what: "sort", .. })
    ));
    // Kind-specific fields must exist and have a fitting type.
    assert!(
        with(&|v| v.kind = ViewKind::Kanban { stack_by: fid(99) }).contains("is not in the table")
    );
    assert!(with(&|v| v.kind = ViewKind::Kanban {
        stack_by: fid(TITLE)
    })
    .contains("text field cannot be the stacking field"));
    assert!(with(&|v| v.kind = ViewKind::Kanban {
        stack_by: fid(TAGS)
    })
    .contains("cannot be the stacking field"));
    assert!(with(&|v| v.kind = ViewKind::Calendar {
        start: fid(TITLE),
        end: None
    })
    .contains("cannot be the start date"));
    assert!(with(&|v| v.kind = ViewKind::Calendar {
        start: fid(CLOSE),
        end: Some(fid(QTY))
    })
    .contains("cannot be the end date"));
    assert!(with(&|v| v.kind = ViewKind::Calendar {
        start: fid(CLOSE),
        end: Some(fid(CLOSE))
    })
    .contains("same field"));
    assert!(with(&|v| v.kind = ViewKind::Timeline {
        start: fid(CLOSE),
        end: fid(CLOSE)
    })
    .contains("same field"));
    assert!(with(&|v| v.kind = ViewKind::Timeline {
        start: fid(CLOSE),
        end: fid(99)
    })
    .contains("is not in the table"));
    assert!(with(&|v| v.kind = ViewKind::Timeline {
        start: fid(DONE),
        end: fid(CLOSE)
    })
    .contains("cannot be the start date"));
    assert!(with(&|v| v.kind = ViewKind::Gallery {
        cover: Some(fid(TITLE))
    })
    .contains("cannot be the cover"));
    // Layout.
    assert!(with(&|v| v.fields = vec![setting(99, true, None)]).contains("is not in the table"));
    assert!(
        with(&|v| v.fields = vec![setting(TITLE, true, None), setting(TITLE, false, None)])
            .contains("listed twice")
    );
    assert!(
        with(&|v| v.fields = vec![setting(TITLE, true, Some(MIN_COLUMN_WIDTH - 1))])
            .contains("column width")
    );
    assert!(
        with(&|v| v.fields = vec![setting(TITLE, true, Some(MAX_COLUMN_WIDTH + 1))])
            .contains("column width")
    );
    assert!(with(&|v| {
        v.fields = vec![setting(TITLE, true, None), setting(QTY, false, None)];
        v.frozen = 2;
    })
    .contains("frozen"));
    assert!(with(&|v| v.frozen = 1).contains("frozen"));
    // The limits themselves are fine.
    let mut v = view();
    v.fields = vec![
        setting(TITLE, true, Some(MIN_COLUMN_WIDTH)),
        setting(QTY, true, Some(MAX_COLUMN_WIDTH)),
    ];
    v.frozen = 2;
    v.name = "x".repeat(MAX_VIEW_NAME_CHARS);
    v.check(&type_of).unwrap();
    // Colour rules.
    let rule = |colour: &str, target: ColourTarget| ColourRule {
        when: Condition::is_empty(fid(TITLE)),
        target,
        colour: colour.into(),
    };
    for bad in ["", "Red", "#ff0000", "red;", &"r".repeat(33)] {
        assert!(
            with(&|v| v.colours = vec![rule(bad, ColourTarget::Record)]).contains("theme token"),
            "{bad}"
        );
    }
    assert!(
        with(&|v| v.colours = vec![rule("red", ColourTarget::Field { field: fid(99) })])
            .contains("coloured field")
    );
    let mut v = view();
    v.colours = vec![ColourRule {
        when: Condition::is_empty(fid(99)),
        target: ColourTarget::Record,
        colour: "red".into(),
    }];
    assert!(matches!(v.check(&type_of), Err(QueryError::Core(_))));
    let mut v = view();
    v.colours = vec![rule("red", ColourTarget::Record); MAX_COLOUR_RULES + 1];
    assert_eq!(
        v.check(&type_of),
        Err(QueryError::Limit("too many colour rules"))
    );
    v.colours.pop();
    v.check(&type_of).unwrap();
}

#[test]
fn std_who_sees_and_who_may_change_a_view() {
    let (owner, other) = (person(1), person(2));
    let with_access = |access: ViewAccess| {
        let mut v = view();
        v.access = access;
        v
    };
    // (access, listed for other, can_save: owner/editor, other/editor, other/manager, other/nobody)
    let cases = [
        (ViewAccess::Shared, true, true, true, true, false),
        (ViewAccess::Personal, false, true, false, false, false),
        (ViewAccess::Protected, true, true, false, true, false),
        (ViewAccess::Locked, true, false, false, false, false),
    ];
    for (access, listed, owner_saves, editor_saves, manager_saves, reader_saves) in cases {
        let v = with_access(access);
        assert!(v.is_listed_for(owner));
        assert_eq!(v.is_listed_for(other), listed, "{access:?}");
        assert_eq!(
            v.can_save(owner, true, false),
            owner_saves,
            "{access:?} owner"
        );
        assert_eq!(
            v.can_save(other, true, false),
            editor_saves,
            "{access:?} editor"
        );
        assert_eq!(
            v.can_save(other, false, true),
            manager_saves,
            "{access:?} manager"
        );
        assert_eq!(
            v.can_save(other, false, false),
            reader_saves,
            "{access:?} reader"
        );
    }
    // The owner of a personal view saves it even without the general capability.
    assert!(with_access(ViewAccess::Personal).can_save(owner, false, false));
    // A locked view is locked for its owner and for managers too, until unlocked.
    assert!(!with_access(ViewAccess::Locked).can_save(owner, true, true));

    // Changing the access mode: owners of their own views, managers of any.
    let cases = [
        (ViewAccess::Shared, false, true),
        (ViewAccess::Personal, true, true),
        (ViewAccess::Protected, true, true),
        (ViewAccess::Locked, false, true),
    ];
    for (access, owner_can, manager_can) in cases {
        let v = with_access(access);
        assert_eq!(
            v.can_change_access(owner, false),
            owner_can,
            "{access:?} owner"
        );
        assert_eq!(
            v.can_change_access(other, true),
            manager_can,
            "{access:?} manager"
        );
        assert!(!v.can_change_access(other, false), "{access:?} other");
    }
}
