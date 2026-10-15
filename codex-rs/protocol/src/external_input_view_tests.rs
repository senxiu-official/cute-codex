use super::*;
use serde_json::json;

#[test]
fn view_bounds_and_canonical_order() {
    let view = View {
        schema: "test.v1".into(),
        data: json!({"z":[true,null,7],"a":"🦀\n"}),
    };
    assert!(view.validate().is_ok());
    assert_eq!(
        view.canonical_json(),
        "{\"data\":{\"a\":\"🦀\\n\",\"z\":[true,null,7]},\"schema\":\"test.v1\"}"
    );
    for data in [
        json!([]),
        json!({"float":1.5}),
        json!({"many":vec![0;1024]}),
        json!({"large":"x".repeat(16384)}),
    ] {
        assert!(
            View {
                data,
                ..view.clone()
            }
            .validate()
            .is_err()
        );
    }
    let mut data = json!({});
    for _ in 0..7 {
        data = json!({"x":data});
    }
    assert!(
        View {
            data: data.clone(),
            ..view.clone()
        }
        .validate()
        .is_ok()
    );
    assert!(
        View {
            data: json!({"x":data}),
            ..view
        }
        .validate()
        .is_err()
    );
}

#[test]
fn view_exact_encoded_bytes_and_entry_limits() {
    let mut view = View {
        schema: "x".into(),
        data: json!({"x":""}),
    };
    let overhead = view.canonical_json().len();
    view.data["x"] = json!("x".repeat(16384 - overhead));
    assert_eq!(view.canonical_json().len(), 16384);
    assert!(view.validate().is_ok());
    view.data["x"] = json!("x".repeat(16385 - overhead));
    assert!(view.validate().is_err());
    view.data = json!({"x":vec![0;1023]});
    assert!(view.validate().is_ok());
    view.data = json!({"x":vec![0;1024]});
    assert!(view.validate().is_err());
}
