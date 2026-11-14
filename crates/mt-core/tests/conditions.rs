//! Property tests of the condition language: the algebra users rely on when they combine
//! filters ("not", "and", "or"), the consistency of the comparison operators, and the calendar
//! arithmetic of relative periods.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use mt_core::condition::{
    fold_text, Condition, Context, Operand, Operator, Period, PeriodUnit, UtcContext,
};
use mt_core::decimal::Decimal;
use mt_core::field::DataType;
use mt_core::id::{FieldId, PrincipalId, RefItemId};
use mt_core::money::{Currency, Money};
use mt_core::time::{Date, DurationMs, Timestamp, Weekday};
use mt_core::value::Value;
use proptest::prelude::*;
use std::collections::BTreeMap;

type Record = BTreeMap<FieldId, Value>;

fn fid(n: u128) -> FieldId {
    FieldId::from_u128(n)
}

fn small_decimal() -> impl Strategy<Value = Decimal> {
    (-50_i64..50, 0_u32..3).prop_map(|(n, scale)| Decimal::new(n, scale))
}

fn small_text() -> impl Strategy<Value = String> {
    prop_oneof!["[a-cA-C ]{0,4}", "[äÄßσΣ]{0,2}[a-b]{0,2}"]
}

fn small_date() -> impl Strategy<Value = Date> {
    (20_580_i32..20_600).prop_map(|days| Date::from_days(days).unwrap())
}

fn small_ids() -> impl Strategy<Value = Vec<u128>> {
    proptest::collection::vec(0_u128..5, 0..4)
}

/// A value of a small domain, so that equal and overlapping values are common.
fn value_of(data_type: DataType) -> BoxedStrategy<Value> {
    match data_type {
        DataType::Text => small_text().prop_map(Value::Text).boxed(),
        DataType::LongText => small_text().prop_map(Value::LongText).boxed(),
        DataType::Number => small_decimal().prop_map(Value::Number).boxed(),
        DataType::Percent => small_decimal().prop_map(Value::Percent).boxed(),
        DataType::Money => (small_decimal(), prop_oneof![Just("USD"), Just("EUR")])
            .prop_map(|(amount, code)| {
                let currency = Currency::new(code).unwrap();
                Value::Money(Money::rounded(
                    amount,
                    currency,
                    mt_core::decimal::Rounding::HalfEven,
                ))
            })
            .boxed(),
        DataType::Bool => any::<bool>().prop_map(Value::Bool).boxed(),
        DataType::Date => small_date().prop_map(Value::Date).boxed(),
        DataType::DateTime => (small_date(), 0_i64..86_400_000)
            .prop_map(|(day, ms)| {
                Value::DateTime(Timestamp::from_millis(day.to_timestamp().millis() + ms).unwrap())
            })
            .boxed(),
        DataType::Duration => (-5_i64..5)
            .prop_map(|m| Value::Duration(DurationMs::from_millis(m * 60_000)))
            .boxed(),
        DataType::Reference => (0_u128..5)
            .prop_map(|n| Value::Reference(RefItemId::from_u128(n)))
            .boxed(),
        DataType::MultiReference => small_ids()
            .prop_map(|ids| {
                Value::MultiReference(ids.into_iter().map(RefItemId::from_u128).collect())
            })
            .boxed(),
        DataType::Link => small_ids()
            .prop_map(|ids| {
                Value::Link(
                    ids.into_iter()
                        .map(mt_core::id::RecordId::from_u128)
                        .collect(),
                )
            })
            .boxed(),
        DataType::Principal => small_ids()
            .prop_map(|ids| Value::Principal(ids.into_iter().map(PrincipalId::from_u128).collect()))
            .boxed(),
        DataType::Attachment => small_ids()
            .prop_map(|ids| {
                Value::Attachment(
                    ids.into_iter()
                        .map(mt_core::id::AttachmentId::from_u128)
                        .collect(),
                )
            })
            .boxed(),
        DataType::Json => prop_oneof![
            Just(serde_json::json!(null)),
            Just(serde_json::json!(1)),
            Just(serde_json::json!({"a": [1, "x"]}))
        ]
        .prop_map(Value::Json)
        .boxed(),
    }
}

fn data_type() -> impl Strategy<Value = DataType> {
    (0..DataType::ALL.len()).prop_map(|index| DataType::ALL[index])
}

fn any_value() -> impl Strategy<Value = Value> {
    prop_oneof![1 => Just(Value::Null), 9 => data_type().prop_flat_map(value_of)]
}

fn operator() -> impl Strategy<Value = Operator> {
    (0..Operator::ALL.len()).prop_map(|index| Operator::ALL[index])
}

fn period() -> impl Strategy<Value = Period> {
    ((0..PeriodUnit::ALL.len()), -3_i32..4, 0_u32..4).prop_map(|(unit, offset, count)| Period {
        unit: PeriodUnit::ALL[unit],
        offset,
        count,
    })
}

/// Any operand at all, well-typed or not.
fn operand() -> impl Strategy<Value = Operand> {
    prop_oneof![
        1 => Just(Operand::None),
        6 => any_value().prop_map(Operand::Value),
        2 => proptest::collection::vec(any_value(), 0..4).prop_map(Operand::List),
        2 => (any_value(), any_value()).prop_map(|(a, b)| Operand::Range(a, b)),
        2 => (1_u128..5).prop_map(|n| Operand::Field(fid(n))),
        1 => Just(Operand::Me),
        2 => period().prop_map(Operand::Period),
    ]
}

fn test_condition() -> impl Strategy<Value = Condition> {
    (1_u128..5, operator(), operand())
        .prop_map(|(field, op, operand)| Condition::test(fid(field), op, operand))
}

/// Any condition at all, up to three levels deep.
fn condition() -> impl Strategy<Value = Condition> {
    test_condition().prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..4).prop_map(Condition::All),
            proptest::collection::vec(inner.clone(), 0..4).prop_map(Condition::Any),
            inner.prop_map(Condition::negated),
        ]
    })
}

fn record() -> impl Strategy<Value = Record> {
    proptest::collection::vec(any_value(), 4).prop_map(|values| {
        values
            .into_iter()
            .enumerate()
            .map(|(index, value)| (fid(u128::try_from(index).unwrap() + 1), value))
            .collect()
    })
}

fn context() -> impl Strategy<Value = UtcContext> {
    (
        small_date(),
        0..7_usize,
        1_u32..=12,
        proptest::option::of(0_u128..5),
    )
        .prop_map(|(today, week_start, fiscal, me)| UtcContext {
            me: me.map(PrincipalId::from_u128),
            today,
            week_start: Weekday::ALL[week_start],
            fiscal_year_start_month: fiscal,
        })
}

fn run(condition: &Condition, record: &Record, ctx: &UtcContext) -> bool {
    condition.evaluate(&|field| record.get(&field), ctx)
}

fn twin(op: Operator) -> Option<Operator> {
    match op {
        Operator::IsEmpty => Some(Operator::IsNotEmpty),
        Operator::Eq => Some(Operator::Ne),
        Operator::Contains => Some(Operator::NotContains),
        Operator::In => Some(Operator::NotIn),
        Operator::HasAny => Some(Operator::HasNone),
        Operator::IsNotEmpty
        | Operator::Ne
        | Operator::NotContains
        | Operator::NotIn
        | Operator::HasNone
        | Operator::Lt
        | Operator::Le
        | Operator::Gt
        | Operator::Ge
        | Operator::Between
        | Operator::StartsWith
        | Operator::EndsWith
        | Operator::HasAll => None,
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 1024, ..ProptestConfig::default() })]

    /// Evaluation is total: no condition, however ill-typed, panics, and the same inputs
    /// always give the same answer.
    #[test]
    fn evaluation_is_total_and_deterministic(c in condition(), r in record(), ctx in context()) {
        prop_assert_eq!(run(&c, &r, &ctx), run(&c, &r, &ctx));
        // Checking never panics either, whatever it concludes.
        let _ = c.check(&|field| r.get(&field).and_then(Value::data_type));
    }

    /// "Is not X" is exactly "not (is X)", for every negative operator and any operand —
    /// including empty fields and ill-typed operands.
    #[test]
    fn negative_operators_negate_their_twins(field in 1_u128..5, op in operator(), o in operand(), r in record(), ctx in context()) {
        let Some(negative) = twin(op) else { return Ok(()) };
        let positive = Condition::test(fid(field), op, o.clone());
        let negated = Condition::test(fid(field), negative, o);
        prop_assert_eq!(run(&negated, &r, &ctx), !run(&positive, &r, &ctx));
    }

    /// De Morgan's laws and double negation hold, so rewriting a filter never changes it.
    #[test]
    fn boolean_algebra_holds(a in condition(), b in condition(), r in record(), ctx in context()) {
        let (va, vb) = (run(&a, &r, &ctx), run(&b, &r, &ctx));
        let and = Condition::All(vec![a.clone(), b.clone()]);
        let or = Condition::Any(vec![a.clone(), b.clone()]);
        prop_assert_eq!(run(&and, &r, &ctx), va && vb);
        prop_assert_eq!(run(&or, &r, &ctx), va || vb);
        let not_and = and.negated();
        let or_of_nots = Condition::Any(vec![a.clone().negated(), b.clone().negated()]);
        prop_assert_eq!(run(&not_and, &r, &ctx), run(&or_of_nots, &r, &ctx));
        let not_or = or.negated();
        let and_of_nots = Condition::All(vec![a.clone().negated(), b.negated()]);
        prop_assert_eq!(run(&not_or, &r, &ctx), run(&and_of_nots, &r, &ctx));
        prop_assert_eq!(run(&a.clone().negated().negated(), &r, &ctx), va);
    }

    /// A condition survives storage: its JSON form evaluates exactly like the original.
    #[test]
    fn serde_round_trip_preserves_meaning(c in condition(), r in record(), ctx in context()) {
        let json = serde_json::to_string(&c).unwrap();
        let back: Condition = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(&back, &c);
        prop_assert_eq!(run(&back, &r, &ctx), run(&c, &r, &ctx));
        prop_assert_eq!(back.fields(), c.fields());
        prop_assert_eq!(back.test_count(), c.test_count());
    }

    /// For two values of an ordered type exactly one of "less", "equal", "greater" holds —
    /// or none, for money in different currencies — and `le` / `ge` / `between` agree.
    #[test]
    fn ordering_operators_are_consistent(
        (a, b) in prop_oneof![
            Just(DataType::Text), Just(DataType::Number), Just(DataType::Money), Just(DataType::Percent),
            Just(DataType::Date), Just(DataType::DateTime), Just(DataType::Duration)
        ].prop_flat_map(|t| (value_of(t), value_of(t))),
        ctx in context(),
    ) {
        if a.is_empty() || b.is_empty() {
            return Ok(());
        }
        let r: Record = [(fid(1), a.clone())].into_iter().collect();
        let holds = |op: Operator| run(&Condition::compare(fid(1), op, b.clone()), &r, &ctx);
        let (lt, eq, gt) = (holds(Operator::Lt), holds(Operator::Eq), holds(Operator::Gt));
        let comparable = match (&a, &b) {
            (Value::Money(x), Value::Money(y)) => x.currency() == y.currency(),
            _ => true,
        };
        let count = [lt, eq, gt].iter().filter(|held| **held).count();
        prop_assert_eq!(count, usize::from(comparable), "lt={} eq={} gt={}", lt, eq, gt);
        prop_assert_eq!(holds(Operator::Le), lt || eq);
        prop_assert_eq!(holds(Operator::Ge), gt || eq);
        prop_assert_eq!(holds(Operator::Ne), !eq);
        // `between x and x` is equality; `between` is "not below the start and not above the end".
        let between = |low: &Value, high: &Value| {
            run(&Condition::test(fid(1), Operator::Between, Operand::Range(low.clone(), high.clone())), &r, &ctx)
        };
        prop_assert_eq!(between(&b, &b), eq);
        prop_assert_eq!(between(&b, &a), holds(Operator::Ge));
    }

    /// Text equality is equality of folded text, and "contains" finds every substring.
    #[test]
    fn text_operators_fold_consistently(haystack in small_text(), from in 0_usize..6, len in 0_usize..6) {
        let chars: Vec<char> = haystack.chars().collect();
        let needle: String = chars.iter().skip(from).take(len).collect();
        let r: Record = [(fid(1), Value::Text(haystack.clone()))].into_iter().collect();
        let ctx = UtcContext::on(Date::EPOCH);
        let holds = |op: Operator, operand: &str| {
            run(&Condition::compare(fid(1), op, Value::Text(operand.to_owned())), &r, &ctx)
        };
        if haystack.trim().is_empty() {
            prop_assert!(!holds(Operator::Contains, &needle));
            return Ok(());
        }
        prop_assert_eq!(holds(Operator::Eq, &haystack.to_uppercase()), fold_text(&haystack) == fold_text(&haystack.to_uppercase()));
        prop_assert!(holds(Operator::Eq, &haystack));
        // Any piece of the text is found, in any case, unless the piece is empty.
        let expected = !fold_text(&needle).is_empty();
        prop_assert_eq!(holds(Operator::Contains, &needle), expected);
        prop_assert_eq!(holds(Operator::Contains, &needle.to_lowercase()), !fold_text(&needle.to_lowercase()).is_empty());
        let prefix: String = chars.iter().take(len).collect();
        prop_assert_eq!(holds(Operator::StartsWith, &prefix), !fold_text(&prefix).is_empty());
    }

    /// List operators behave like set operations.
    #[test]
    fn list_operators_are_set_operations(mine in small_ids(), theirs in small_ids()) {
        use std::collections::BTreeSet;
        let refs = |ids: &[u128]| Value::MultiReference(ids.iter().copied().map(RefItemId::from_u128).collect());
        let r: Record = [(fid(1), refs(&mine))].into_iter().collect();
        let ctx = UtcContext::on(Date::EPOCH);
        let holds = |op: Operator| run(&Condition::compare(fid(1), op, refs(&theirs)), &r, &ctx);
        let a: BTreeSet<u128> = mine.iter().copied().collect();
        let b: BTreeSet<u128> = theirs.iter().copied().collect();
        prop_assert_eq!(holds(Operator::HasAny), !a.is_disjoint(&b));
        prop_assert_eq!(holds(Operator::HasNone), a.is_disjoint(&b));
        prop_assert_eq!(holds(Operator::HasAll), !a.is_empty() && !b.is_empty() && b.is_subset(&a));
        prop_assert_eq!(holds(Operator::Eq), !a.is_empty() && a == b);
    }

    /// Periods are whole, ordered spans; the current unit contains today; and consecutive
    /// units tile the calendar without gaps or overlaps.
    #[test]
    fn periods_tile_the_calendar(unit in 0..PeriodUnit::ALL.len(), offset in -30_i32..30, count in 1_u32..5, ctx in context()) {
        let unit = PeriodUnit::ALL[unit];
        let (first, last) = Period { unit, offset, count }.resolve(&ctx).unwrap();
        prop_assert!(first <= last);
        let (this_first, this_last) = Period::this(unit).resolve(&ctx).unwrap();
        prop_assert!(this_first <= ctx.today() && ctx.today() <= this_last);
        // The unit after this span starts the day after it ends.
        let following = Period { unit, offset: offset + i32::try_from(count).unwrap(), count: 1 };
        let (next_first, _) = following.resolve(&ctx).unwrap();
        prop_assert_eq!(next_first, last.checked_add_days(1).unwrap());
        // A span of `count` units is the union of its single units.
        let (single_first, _) = Period { unit, offset, count: 1 }.resolve(&ctx).unwrap();
        prop_assert_eq!(single_first, first);
        // Month-based units start on the first of a month; weeks on the context's week start.
        match unit {
            PeriodUnit::Day => prop_assert_eq!(i64::from(last.days() - first.days()), i64::from(count) - 1),
            PeriodUnit::Week => {
                prop_assert_eq!(first.weekday(), ctx.week_start());
                prop_assert_eq!(i64::from(last.days() - first.days()), i64::from(count) * 7 - 1);
            }
            PeriodUnit::Month | PeriodUnit::Quarter | PeriodUnit::Year => prop_assert_eq!(first.day(), 1),
            PeriodUnit::FiscalQuarter | PeriodUnit::FiscalYear => {
                prop_assert_eq!(first.day(), 1);
                let months_from_start = (i64::from(first.month()) - i64::from(ctx.fiscal_year_start_month())).rem_euclid(12);
                let size = if unit == PeriodUnit::FiscalYear { 12 } else { 3 };
                prop_assert_eq!(months_from_start % size, 0);
            }
        }
    }

    /// A date field tested against a period agrees with plain date arithmetic.
    #[test]
    fn period_tests_agree_with_date_arithmetic(day in small_date(), p in period(), ctx in context()) {
        let r: Record = [(fid(1), Value::Date(day))].into_iter().collect();
        let holds = |op: Operator| run(&Condition::test(fid(1), op, Operand::Period(p)), &r, &ctx);
        match p.resolve(&ctx) {
            None => {
                for op in [Operator::Eq, Operator::Lt, Operator::Le, Operator::Gt, Operator::Ge, Operator::Between] {
                    prop_assert!(!holds(op));
                }
                prop_assert!(holds(Operator::Ne));
            }
            Some((first, last)) => {
                prop_assert_eq!(holds(Operator::Eq), first <= day && day <= last);
                prop_assert_eq!(holds(Operator::Between), first <= day && day <= last);
                prop_assert_eq!(holds(Operator::Lt), day < first);
                prop_assert_eq!(holds(Operator::Le), day <= last);
                prop_assert_eq!(holds(Operator::Gt), day > last);
                prop_assert_eq!(holds(Operator::Ge), day >= first);
                // A date-time at any moment of that day (UTC context) gives the same answers.
                let noon = Timestamp::from_millis(day.to_timestamp().millis() + 43_200_000).unwrap();
                let r2: Record = [(fid(1), Value::DateTime(noon))].into_iter().collect();
                for op in [Operator::Eq, Operator::Lt, Operator::Le, Operator::Gt, Operator::Ge] {
                    let at_noon = run(&Condition::test(fid(1), op, Operand::Period(p)), &r2, &ctx);
                    prop_assert_eq!(at_noon, holds(op), "{}", op);
                }
            }
        }
    }

    /// A condition accepted by `check` reads only fields it declares, and declaring more
    /// fields never changes the answer.
    #[test]
    fn evaluation_reads_only_declared_fields(c in condition(), r in record(), extra in any_value(), ctx in context()) {
        let declared = c.fields();
        let mut widened = r.clone();
        for n in 1_u128..5 {
            if !declared.contains(&fid(n)) {
                widened.insert(fid(n), extra.clone());
            }
        }
        prop_assert_eq!(run(&c, &widened, &ctx), run(&c, &r, &ctx));
    }
}
