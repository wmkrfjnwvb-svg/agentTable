//! The parameter registry: definitions, scoped overrides, resolution with an explanation,
//! secrets, feature flags, history and option lists for fields.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::*;
use mt_core::decimal::Decimal;
use mt_core::field::{Classification, DataType, FieldDef, OptionsSource, ValidationSpec};
use mt_core::id::{BaseId, FieldId, RefSetId};
use mt_core::time::DurationMs;
use mt_core::validate::{validate_value, AcceptAllOptions, IssueCode};
use mt_core::value::Value;
use mt_refdata::params::{check_key, Entry, MAX_HISTORY, MAX_JSON_BYTES, MAX_KEY_CHARS};
use mt_refdata::{
    EffectiveSet, NoParameters, ParamContext, ParamDef, ParamKind, ParameterOptions, RefData,
    RefDataError, Registry, Scope, ScopeLevel, SecretRef, Setting, Source,
};

fn dec(text: &str) -> Decimal {
    text.parse().unwrap()
}

fn num(text: &str) -> Value {
    Value::Number(dec(text))
}

fn base(n: u128) -> BaseId {
    BaseId::from_u128(n)
}

const ALL_LEVELS: [ScopeLevel; 3] = [ScopeLevel::Tenant, ScopeLevel::Base, ScopeLevel::User];

fn registry() -> Registry {
    let mut registry = Registry::new();
    let any = AcceptAllOptions;
    registry
        .define(
            ParamDef::integer(
                "approval.timeout_hours",
                "Hours before an approval escalates",
            )
            .with_default(num("48"))
            .with_range(Some(dec("1")), Some(dec("720")))
            .overridable(&ALL_LEVELS),
            &any,
        )
        .unwrap();
    registry
        .define(
            ParamDef::flag("feature.new_grid", "Use the new grid").overridable(&ALL_LEVELS),
            &any,
        )
        .unwrap();
    registry
        .define(
            ParamDef::text("ui.greeting", "Greeting on the home page"),
            &any,
        )
        .unwrap();
    registry
        .define(
            ParamDef::secret("mail.smtp_password", "Password of the SMTP relay"),
            &any,
        )
        .unwrap();
    registry
        .define(
            ParamDef::duration("sla.first_response", "Target time to first response")
                .with_default(Value::Duration(DurationMs::from_millis(3_600_000))),
            &any,
        )
        .unwrap();
    registry
}

fn value(v: Value) -> Setting {
    Setting::Value(v)
}

#[test]
fn std_resolution_takes_the_most_specific_value_and_explains_it() {
    let mut registry = registry();
    let any = AcceptAllOptions;
    let key = "approval.timeout_hours";
    let (admin, lead, user) = (person(1), person(2), person(3));
    let nobody = ParamContext::default();
    let in_base = ParamContext {
        base: Some(base(7)),
        user: None,
    };
    let as_user = ParamContext {
        base: Some(base(7)),
        user: Some(user),
    };

    // Default.
    let resolved = registry.resolve(key, &nobody).unwrap();
    assert_eq!(
        (resolved.value, resolved.source),
        (Some(&num("48")), Source::Default)
    );
    assert_eq!(
        resolved.explain(),
        "`approval.timeout_hours` is 48: the default"
    );

    // Tenant override.
    registry
        .set(key, Scope::Tenant, value(num("24")), admin, at(1_000), &any)
        .unwrap();
    let resolved = registry.resolve(key, &as_user).unwrap();
    assert_eq!(resolved.value, Some(&num("24")));
    assert_eq!(
        resolved.source,
        Source::Override {
            scope: Scope::Tenant,
            by: admin,
            at: at(1_000)
        }
    );
    assert!(resolved
        .explain()
        .starts_with("`approval.timeout_hours` is 24: set for the tenant by prn_"));
    assert!(
        resolved.explain().ends_with("at 1970-01-01T00:00:01.000Z"),
        "{}",
        resolved.explain()
    );

    // Base override wins inside that base only.
    registry
        .set(
            key,
            Scope::Base { base: base(7) },
            value(num("12")),
            lead,
            at(2_000),
            &any,
        )
        .unwrap();
    assert_eq!(registry.number(key, &in_base), Some(dec("12")));
    assert_eq!(
        registry.number(
            key,
            &ParamContext {
                base: Some(base(8)),
                user: None
            }
        ),
        Some(dec("24"))
    );
    assert_eq!(registry.number(key, &nobody), Some(dec("24")));
    assert!(registry
        .resolve(key, &in_base)
        .unwrap()
        .explain()
        .contains("set for the base bas_"));

    // User override wins for that person, in any base.
    registry
        .set(
            key,
            Scope::User { user },
            value(num("6")),
            user,
            at(3_000),
            &any,
        )
        .unwrap();
    assert_eq!(registry.number(key, &as_user), Some(dec("6")));
    assert_eq!(
        registry.number(
            key,
            &ParamContext {
                base: Some(base(8)),
                user: Some(user)
            }
        ),
        Some(dec("6"))
    );
    assert_eq!(
        registry.number(
            key,
            &ParamContext {
                base: Some(base(7)),
                user: Some(person(9))
            }
        ),
        Some(dec("12"))
    );
    assert!(registry
        .resolve(key, &as_user)
        .unwrap()
        .explain()
        .contains("set for the user prn_"));

    // Clearing an override lets the next level apply again.
    assert!(registry
        .clear(key, Scope::User { user }, user, at(4_000))
        .unwrap());
    assert_eq!(registry.number(key, &as_user), Some(dec("12")));
    assert!(
        !registry
            .clear(key, Scope::User { user }, user, at(4_001))
            .unwrap(),
        "nothing left to clear"
    );
    assert!(registry
        .clear(key, Scope::Base { base: base(7) }, lead, at(5_000))
        .unwrap());
    assert!(registry
        .clear(key, Scope::Tenant, admin, at(6_000))
        .unwrap());
    assert_eq!(
        registry.resolve(key, &as_user).unwrap().source,
        Source::Default
    );

    // History keeps every change, including the clearing.
    let history = registry.history(key, Scope::Tenant);
    assert_eq!(history.len(), 2);
    assert_eq!(
        history[0],
        Entry {
            setting: Some(value(num("24"))),
            by: admin,
            at: at(1_000)
        }
    );
    assert_eq!(
        history[1],
        Entry {
            setting: None,
            by: admin,
            at: at(6_000)
        }
    );
    assert!(registry
        .history(key, Scope::Base { base: base(99) })
        .is_empty());
    assert!(registry.history("no.such", Scope::Tenant).is_empty());
}

#[test]
fn std_unset_parameters_flags_and_typed_getters() {
    let mut registry = registry();
    let any = AcceptAllOptions;
    let ctx = ParamContext::default();
    // No default and no override.
    let unset = registry.resolve("ui.greeting", &ctx).unwrap();
    assert!(!unset.is_set());
    assert_eq!(unset.source, Source::Unset);
    assert_eq!(
        unset.explain(),
        "`ui.greeting` has no value: there is no default and no override"
    );
    assert_eq!(registry.text("ui.greeting", &ctx), None);

    // Feature flags are off until set, and per base.
    assert!(!registry.flag("feature.new_grid", &ctx));
    registry
        .set(
            "feature.new_grid",
            Scope::Base { base: base(1) },
            value(Value::Bool(true)),
            person(1),
            at(1),
            &any,
        )
        .unwrap();
    assert!(registry.flag(
        "feature.new_grid",
        &ParamContext {
            base: Some(base(1)),
            user: None
        }
    ));
    assert!(!registry.flag(
        "feature.new_grid",
        &ParamContext {
            base: Some(base(2)),
            user: None
        }
    ));
    registry
        .set(
            "feature.new_grid",
            Scope::Tenant,
            value(Value::Bool(true)),
            person(1),
            at(2),
            &any,
        )
        .unwrap();
    registry
        .set(
            "feature.new_grid",
            Scope::Base { base: base(2) },
            value(Value::Bool(false)),
            person(1),
            at(3),
            &any,
        )
        .unwrap();
    assert!(registry.flag("feature.new_grid", &ctx));
    assert!(
        !registry.flag(
            "feature.new_grid",
            &ParamContext {
                base: Some(base(2)),
                user: None
            }
        ),
        "a base can opt out"
    );
    // Unknown and non-boolean parameters are simply off.
    assert!(!registry.flag("no.such", &ctx));
    assert!(!registry.flag("approval.timeout_hours", &ctx));

    registry
        .set(
            "ui.greeting",
            Scope::Tenant,
            value(Value::Text("Welcome".into())),
            person(1),
            at(4),
            &any,
        )
        .unwrap();
    assert_eq!(registry.text("ui.greeting", &ctx), Some("Welcome"));
    assert_eq!(
        registry.duration("sla.first_response", &ctx),
        Some(DurationMs::from_millis(3_600_000))
    );
    assert_eq!(registry.number("ui.greeting", &ctx), None);
    assert_eq!(registry.duration("ui.greeting", &ctx), None);
    assert_eq!(registry.text("approval.timeout_hours", &ctx), None);
    assert_eq!(registry.value("no.such", &ctx), None);
    assert_eq!(
        registry.resolve("no.such", &ctx).unwrap_err(),
        RefDataError::UnknownParameter {
            key: "no.such".into()
        }
    );
    assert_eq!(registry.definitions().count(), 5);
    assert!(registry.definition("ui.greeting").is_some());
}

#[test]
fn std_secrets_are_write_only() {
    let mut registry = registry();
    let any = AcceptAllOptions;
    let ctx = ParamContext::default();
    let key = "mail.smtp_password";
    assert!(registry.definition(key).unwrap().is_secret());
    assert_eq!(
        registry.definition(key).unwrap().sensitivity,
        Classification::Confidential
    );
    assert!(!registry.resolve(key, &ctx).unwrap().is_set());

    let handle = SecretRef::new("vault:tenant-a/smtp/v3").unwrap();
    registry
        .set(
            key,
            Scope::Tenant,
            Setting::Secret(handle.clone()),
            person(1),
            at(1),
            &any,
        )
        .unwrap();
    let resolved = registry.resolve(key, &ctx).unwrap();
    // The value is never exposed; only the handle, to code that asks for it explicitly.
    assert_eq!(resolved.value, None);
    assert_eq!(
        resolved.secret.map(SecretRef::handle),
        Some("vault:tenant-a/smtp/v3")
    );
    assert!(resolved.is_set());
    assert_eq!(registry.value(key, &ctx), None);
    assert_eq!(registry.text(key, &ctx), None);
    // Neither the explanation nor debug output shows the handle.
    let explanation = resolved.explain();
    assert!(explanation
        .starts_with("`mail.smtp_password` is a secret that is set: set for the tenant by"));
    assert!(!explanation.contains("vault"));
    assert_eq!(format!("{handle:?}"), "SecretRef(<redacted>)");
    assert!(!format!("{resolved:?}").contains("vault"));
    assert!(!format!("{registry:?}").contains("vault"));

    // A secret takes a handle, never a plain value; other parameters never take a handle.
    assert!(matches!(
        registry.set(
            key,
            Scope::Tenant,
            value(Value::Text("hunter2".into())),
            person(1),
            at(2),
            &any
        ),
        Err(RefDataError::InvalidParameter { .. })
    ));
    assert!(matches!(
        registry.set(
            "ui.greeting",
            Scope::Tenant,
            Setting::Secret(handle),
            person(1),
            at(2),
            &any
        ),
        Err(RefDataError::InvalidParameter { .. })
    ));
    assert!(SecretRef::new("").is_err());
    assert!(SecretRef::new("x".repeat(256)).is_err());
    assert!(matches!(
        ParamDef::secret("a.b", "x")
            .with_default(Value::Text("p".into()))
            .check(&any),
        Err(RefDataError::InvalidParameter { .. })
    ));
}

#[test]
fn invalid_values_and_scopes_are_refused_without_side_effects() {
    let mut registry = registry();
    let any = AcceptAllOptions;
    let before = registry.clone();
    let key = "approval.timeout_hours";
    let set = |registry: &mut Registry, key: &str, scope: Scope, v: Value| {
        registry.set(key, scope, value(v), person(1), at(1), &any)
    };
    let cases: Vec<(&str, Scope, Value, &str)> = vec![
        (key, Scope::Tenant, num("0"), "not acceptable"),
        (key, Scope::Tenant, num("721"), "not acceptable"),
        (key, Scope::Tenant, num("1.5"), "not acceptable"),
        (
            key,
            Scope::Tenant,
            Value::Text("48".into()),
            "not acceptable",
        ),
        (
            key,
            Scope::Tenant,
            Value::Null,
            "clear the override instead",
        ),
        (
            "ui.greeting",
            Scope::Tenant,
            Value::Text("   ".into()),
            "clear the override instead",
        ),
        (
            "feature.new_grid",
            Scope::Tenant,
            num("1"),
            "not acceptable",
        ),
    ];
    for (key, scope, v, expected) in cases {
        let err = set(&mut registry, key, scope, v.clone()).unwrap_err();
        assert!(err.to_string().contains(expected), "{key} = {v:?}: {err}");
        assert_eq!(registry, before);
    }
    // The limits themselves are allowed.
    set(&mut registry, key, Scope::Tenant, num("1")).unwrap();
    set(&mut registry, key, Scope::Tenant, num("720")).unwrap();
    // Not every parameter can be overridden everywhere.
    assert_eq!(
        set(
            &mut registry,
            "ui.greeting",
            Scope::Base { base: base(1) },
            Value::Text("Hi".into())
        ),
        Err(RefDataError::ScopeNotAllowed {
            key: "ui.greeting".into(),
            scope: "base"
        })
    );
    assert_eq!(
        set(
            &mut registry,
            "ui.greeting",
            Scope::User { user: person(1) },
            Value::Text("Hi".into())
        ),
        Err(RefDataError::ScopeNotAllowed {
            key: "ui.greeting".into(),
            scope: "user"
        })
    );
    assert_eq!(
        set(&mut registry, "no.such", Scope::Tenant, num("1")),
        Err(RefDataError::UnknownParameter {
            key: "no.such".into()
        })
    );
    assert_eq!(
        registry.clear("no.such", Scope::Tenant, person(1), at(1)),
        Err(RefDataError::UnknownParameter {
            key: "no.such".into()
        })
    );
    assert_eq!(Scope::Tenant.level(), ScopeLevel::Tenant);
    assert_eq!(ScopeLevel::User.name(), "user");
}

#[test]
fn invalid_definitions() {
    let any = AcceptAllOptions;
    for key in ["a.b", "import.max_rows", "ai.provider.default", "x1.y_2"] {
        check_key(key).unwrap();
    }
    let long = format!("a.{}", "b".repeat(MAX_KEY_CHARS));
    for key in [
        "",
        "nodots",
        "A.b",
        "a..b",
        ".a",
        "a.",
        "a.1b",
        "a b.c",
        "a-b.c",
        long.as_str(),
    ] {
        assert!(check_key(key).is_err(), "{key}");
    }
    let check = |def: ParamDef| def.check(&any).unwrap_err().to_string();
    assert!(check(ParamDef::text("Bad Key", "x")).contains("parameter key"));
    assert!(check(ParamDef::text("a.b", "  ")).contains("parameter description"));
    assert!(check(ParamDef::text("a.b", &"d".repeat(1_001))).contains("parameter description"));
    // The default must satisfy the definition.
    assert!(
        check(ParamDef::integer("a.b", "x").with_default(num("1.5"))).contains("not acceptable")
    );
    assert!(check(
        ParamDef::integer("a.b", "x")
            .with_range(Some(dec("5")), None)
            .with_default(num("4"))
    )
    .contains("not acceptable"));
    // A range that makes no sense is caught by the field framework.
    assert!(matches!(
        ParamDef::integer("a.b", "x")
            .with_range(Some(dec("5")), Some(dec("1")))
            .check(&any),
        Err(RefDataError::Core(_))
    ));
    // Types that cannot be parameters.
    for data_type in [DataType::Link, DataType::Attachment] {
        let def = ParamDef {
            kind: ParamKind::Value {
                data_type,
                set: None,
                validation: Box::default(),
            },
            ..ParamDef::text("a.b", "x")
        };
        assert!(check(def).contains("cannot be of type"));
    }
    // Choice parameters name a set; others must not.
    let def = ParamDef {
        kind: ParamKind::Value {
            data_type: DataType::Reference,
            set: None,
            validation: Box::default(),
        },
        ..ParamDef::text("a.b", "x")
    };
    assert!(check(def).contains("name a reference set"));
    let def = ParamDef {
        kind: ParamKind::Value {
            data_type: DataType::Text,
            set: Some(RefSetId::from_u128(1)),
            validation: Box::default(),
        },
        ..ParamDef::text("a.b", "x")
    };
    assert!(check(def).contains("name a reference set"));
    // JSON parameters have a size limit.
    let mut registry = Registry::new();
    registry
        .define(ParamDef::json("a.shape", "A JSON setting"), &any)
        .unwrap();
    let big = Value::Json(serde_json::json!({ "text": "x".repeat(MAX_JSON_BYTES) }));
    assert!(registry
        .set("a.shape", Scope::Tenant, value(big), person(1), at(1), &any)
        .unwrap_err()
        .to_string()
        .contains("JSON larger than"));
    registry
        .set(
            "a.shape",
            Scope::Tenant,
            value(Value::Json(serde_json::json!({"a": [1, 2]}))),
            person(1),
            at(1),
            &any,
        )
        .unwrap();
    let def = ParamDef::decimal("a.rate", "A rate").with_default(num("0.5"));
    let json = serde_json::to_string(&def).unwrap();
    assert_eq!(serde_json::from_str::<ParamDef>(&json).unwrap(), def);
}

#[test]
fn std_redefining_a_parameter_keeps_existing_overrides_valid() {
    let mut registry = registry();
    let any = AcceptAllOptions;
    let key = "approval.timeout_hours";
    registry
        .set(
            key,
            Scope::Tenant,
            value(num("100")),
            person(1),
            at(1),
            &any,
        )
        .unwrap();
    registry
        .set(
            key,
            Scope::Base { base: base(1) },
            value(num("10")),
            person(1),
            at(2),
            &any,
        )
        .unwrap();

    // Widening is fine.
    let wider = ParamDef::integer(key, "Hours")
        .with_default(num("48"))
        .with_range(Some(dec("1")), Some(dec("1000")))
        .overridable(&ALL_LEVELS);
    registry.define(wider, &any).unwrap();
    // Narrowing below a current override is refused.
    let narrower = ParamDef::integer(key, "Hours")
        .with_default(num("48"))
        .with_range(Some(dec("1")), Some(dec("50")))
        .overridable(&ALL_LEVELS);
    assert!(registry
        .define(narrower, &any)
        .unwrap_err()
        .to_string()
        .contains("not acceptable"));
    // Removing a level that has an override is refused until the override is cleared.
    let tenant_only = ParamDef::integer(key, "Hours")
        .with_default(num("48"))
        .with_range(Some(dec("1")), Some(dec("1000")));
    assert!(registry
        .define(tenant_only.clone(), &any)
        .unwrap_err()
        .to_string()
        .contains("override exists at the base level"));
    registry
        .clear(key, Scope::Base { base: base(1) }, person(1), at(3))
        .unwrap();
    registry.define(tenant_only, &any).unwrap();
    assert_eq!(
        registry.number(
            key,
            &ParamContext {
                base: Some(base(1)),
                user: None
            }
        ),
        Some(dec("100"))
    );
    // An override that was cleared does not stand in the way of a narrower definition.
    registry
        .clear(key, Scope::Tenant, person(1), at(4))
        .unwrap();
    let narrow = ParamDef::integer(key, "Hours")
        .with_default(num("48"))
        .with_range(Some(dec("1")), Some(dec("50")));
    registry.define(narrow, &any).unwrap();
    assert_eq!(
        registry.number(key, &ParamContext::default()),
        Some(dec("48"))
    );
    assert_eq!(ScopeLevel::Tenant.name(), "tenant");
    assert_eq!(ScopeLevel::Base.name(), "base");
    // The type cannot change.
    assert!(registry
        .define(ParamDef::text(key, "Hours"), &any)
        .unwrap_err()
        .to_string()
        .contains("type of a parameter cannot change"));
    assert!(registry
        .define(ParamDef::secret(key, "Hours"), &any)
        .unwrap_err()
        .to_string()
        .contains("type of a parameter cannot change"));
}

#[test]
fn edge_number_of_parameters_is_bounded() {
    use mt_refdata::params::MAX_PARAMETERS;
    let any = AcceptAllOptions;
    let mut registry = Registry::new();
    for n in 0..MAX_PARAMETERS {
        registry
            .define(ParamDef::flag(&format!("f.p{n}"), "flag"), &any)
            .unwrap();
    }
    assert_eq!(
        registry.define(ParamDef::flag("f.one_more", "flag"), &any),
        Err(RefDataError::Limit("too many parameters"))
    );
    // Replacing an existing definition is still possible at the limit.
    registry
        .define(ParamDef::flag("f.p0", "another description"), &any)
        .unwrap();
    assert_eq!(registry.definitions().count(), MAX_PARAMETERS);
}

#[test]
fn edge_history_is_bounded() {
    let mut registry = registry();
    let any = AcceptAllOptions;
    for n in 0..MAX_HISTORY + 25 {
        let hours = Value::Number(Decimal::from(1 + (n % 700) as u32));
        registry
            .set(
                "approval.timeout_hours",
                Scope::Tenant,
                value(hours),
                person(1),
                at(i64::try_from(n).unwrap()),
                &any,
            )
            .unwrap();
    }
    let history = registry.history("approval.timeout_hours", Scope::Tenant);
    assert_eq!(history.len(), MAX_HISTORY);
    // The newest entries are kept.
    assert_eq!(
        history.last().unwrap().at,
        at(i64::try_from(MAX_HISTORY + 24).unwrap())
    );
    assert_eq!(history[0].at, at(25));
}

#[test]
fn std_choice_parameters_use_reference_data_and_feed_field_options() {
    // Reference data: the region tree.
    let mut data = RefData::new();
    data.insert(EffectiveSet::build(&region_set(), &region_items(), None).unwrap())
        .unwrap();
    let on = day(2025, 6, 1);
    let resolver = data.resolver(on, &NoParameters);
    let region = RefSetId::from_u128(REGION);

    let mut registry = Registry::new();
    registry
        .define(
            ParamDef::choice("sales.home_region", "The home region", region),
            &resolver,
        )
        .unwrap();
    registry
        .define(
            ParamDef::choices("sales.active_regions", "Regions we sell in", region)
                .overridable(&[ScopeLevel::Tenant, ScopeLevel::Base]),
            &resolver,
        )
        .unwrap();

    // Values must be selectable items of the set on the effective date.
    registry
        .set(
            "sales.home_region",
            Scope::Tenant,
            value(Value::Reference(rid(11))),
            person(1),
            at(1),
            &resolver,
        )
        .unwrap();
    for bad in [rid(9999), rid(112), rid(31)] {
        assert!(matches!(
            registry.set(
                "sales.home_region",
                Scope::Tenant,
                value(Value::Reference(bad)),
                person(1),
                at(2),
                &resolver
            ),
            Err(RefDataError::InvalidParameter { .. })
        ));
    }
    registry
        .set(
            "sales.active_regions",
            Scope::Tenant,
            value(Value::MultiReference(vec![rid(11), rid(21)])),
            person(1),
            at(3),
            &resolver,
        )
        .unwrap();
    registry
        .set(
            "sales.active_regions",
            Scope::Base { base: base(5) },
            value(Value::MultiReference(vec![rid(21)])),
            person(1),
            at(4),
            &resolver,
        )
        .unwrap();
    assert!(matches!(
        registry.set(
            "sales.active_regions",
            Scope::Tenant,
            value(Value::MultiReference(vec![rid(11), rid(112)])),
            person(1),
            at(5),
            &resolver
        ),
        Err(RefDataError::InvalidParameter { .. })
    ));
    // A default outside the set is refused at definition time.
    assert!(registry
        .define(
            ParamDef::choice("sales.fallback", "Fallback", region)
                .with_default(Value::Reference(rid(9999))),
            &resolver
        )
        .is_err());

    // The list parameter feeds a field's options, per context.
    let tenant_view = registry.bind(ParamContext::default());
    let base_view = registry.bind(ParamContext {
        base: Some(base(5)),
        user: None,
    });
    assert!(tenant_view.offers("sales.active_regions", rid(11)));
    assert!(tenant_view.offers("sales.active_regions", rid(21)));
    assert!(!tenant_view.offers("sales.active_regions", rid(12)));
    assert!(
        !base_view.offers("sales.active_regions", rid(11)),
        "the base narrowed the list"
    );
    assert!(base_view.offers("sales.active_regions", rid(21)));
    // A single-choice parameter offers exactly its item; anything else offers nothing.
    assert!(tenant_view.offers("sales.home_region", rid(11)));
    assert!(!tenant_view.offers("sales.home_region", rid(21)));
    assert!(!tenant_view.offers("no.such", rid(11)));
    assert!(!NoParameters.offers("sales.active_regions", rid(11)));

    // End to end: a field whose options come from the parameter.
    let mut field = FieldDef::new(FieldId::from_u128(1), "Region", DataType::Reference);
    field.options = OptionsSource::Parameter {
        key: "sales.active_regions".into(),
    };
    field.validation = ValidationSpec::default();
    field.check().unwrap();
    let with_params = data.resolver(on, &base_view);
    let codes = |v: Value| {
        validate_value(&field, &v, None, &with_params)
            .into_iter()
            .map(|i| i.code)
            .collect::<Vec<_>>()
    };
    assert!(codes(Value::Reference(rid(21))).is_empty());
    assert_eq!(
        codes(Value::Reference(rid(11))),
        vec![IssueCode::OptionNotAllowed]
    );
}
