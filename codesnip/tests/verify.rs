use std::process::{Command, Output};

fn run(source: &str, args: &[&str]) -> Output {
    run_with_config(source, args, serde_json::json!({}))
}

fn run_with_config(source: &str, args: &[&str], extra: serde_json::Value) -> Output {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, source).unwrap();
    let config = dir.path().join("codesnip.json");
    let mut settings = serde_json::json!({
            "sources": [{ "path": path }],
            "cfg_enable": ["nightly"],
            "cfg_disable": ["test"],
            "filter_attr": ["inline", "doc"],
            "filter_item": ["test"],
            "format": "minify"
    });
    settings
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    std::fs::write(&config, settings.to_string()).unwrap();
    Command::new(env!("CARGO_BIN_EXE_cargo-codesnip"))
        .arg("codesnip")
        .arg("--source-config")
        .arg(config)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn verify_uses_source_config_and_cli_overrides() {
    let source = r#"
        #[codesnip::entry]
        pub fn sample() {
            #[cfg(feature = "extra")]
            let _: () = 1;
        }
    "#;
    let config = serde_json::json!({ "rustc_args": ["--cfg=feature=\"extra\""] });
    let output = run_with_config(source, &["verify"], config.clone());
    assert!(!output.status.success(), "{:?}", output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("E0308"));
    let output = run_with_config(source, &["verify", "--rustc-arg=-Copt-level=0"], config);
    assert!(output.status.success(), "{:?}", output);

    let source = "#[codesnip::entry] pub fn sample() { let unused = 1; }";
    for (deny_warnings, args, success) in [
        (false, vec!["verify"], true),
        (true, vec!["verify"], false),
        (false, vec!["verify", "--deny-warnings"], false),
    ] {
        let output = run_with_config(
            source,
            &args,
            serde_json::json!({ "deny_warnings": deny_warnings }),
        );
        assert_eq!(output.status.success(), success, "{:?}", output);
        if !success {
            assert!(String::from_utf8_lossy(&output.stderr).contains("unused_variables"));
        }
    }
}

#[test]
fn edition_applies_to_formatting_and_verification() {
    let source = "#[codesnip::entry] pub fn gen() {}";
    let config = serde_json::json!({ "format": "rustfmt" });
    let output = run_with_config(
        source,
        &["bundle", "gen", "--edition", "2021"],
        config.clone(),
    );
    assert!(output.status.success(), "{:?}", output);
    assert_eq!(output.stdout, b"// codesnip-guard: gen\npub fn gen() {}\n");
    let output = run_with_config(
        source,
        &["--edition", "2021", "verify", "--deny-warnings"],
        config,
    );
    assert!(output.status.success(), "{:?}", output);
}

#[test]
fn warnings_fail_only_when_requested_and_are_displayed() {
    let source = "#[codesnip::entry] pub fn sample() { let unused = 1; }";
    let output = run(source, &["verify", "--verbose"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(stderr.contains("unused_variables"), "{stderr}");

    let output = run(source, &["verify", "--deny-warnings"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("unused_variables"), "{stderr}");

    let output = run(
        "#[codesnip::entry] #[allow(unused_variables)] pub fn sample() { let unused = 1; }",
        &["verify", "--deny-warnings"],
    );
    assert!(output.status.success(), "{:?}", output);

    let output = run(
        source,
        &[
            "verify",
            "--deny-warnings",
            "--rustc-arg=--force-warn=unused_variables",
        ],
    );
    assert!(!output.status.success(), "{:?}", output);
}

#[test]
fn rustc_arguments_select_the_code_being_verified() {
    let source = r#"
        #[codesnip::entry]
        pub fn sample() {
            #[cfg(feature = "extra")]
            let _: () = 1;
            #[cfg(not(debug_assertions))]
            let _: () = 2;
        }
    "#;
    let output = run(source, &["verify", "--deny-warnings"]);
    assert!(output.status.success(), "{:?}", output);
    for args in [
        vec![
            "verify",
            "--rustc-arg=--cfg",
            "--rustc-arg=feature=\"extra\"",
        ],
        vec!["verify", "--rustc-arg=-Cdebug-assertions=off"],
    ] {
        let output = run(source, &args);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{stderr}");
        assert!(stderr.contains("E0308"), "{stderr}");
    }
    let output = run(source, &["verify", "--rustc-arg=--invalid-codesnip-option"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid-codesnip-option"));
}

#[test]
fn filters_reach_nested_attributes_and_items() {
    let source = r#"
        #[codesnip::entry("sample", inline)]
        mod sample {
            #[inline] pub fn outer() {}
            pub struct S { #[doc = "remove"] pub value: u8 }
            impl S {
                #[inline] pub fn method(&self) {}
                #[codesnip::skip] fn removed_impl() { missing(); }
            }
            pub trait T {
                #[inline] fn method(&self) {}
                #[test] fn removed_trait() { missing(); }
            }
            unsafe extern "C" {
                #[doc = "remove"] pub fn kept_foreign();
                #[codesnip::skip] pub fn removed_foreign();
            }
            pub fn local() {
                #[inline] fn nested() {}
                #[codesnip::skip] fn removed_local() { missing(); }
                nested();
            }
            #[cfg_attr(feature = "extra", inline, must_use)]
            pub fn conditional() -> u8 { 0 }
        }
    "#;
    let output = run(source, &["bundle", "sample"]);
    assert!(output.status.success(), "{:?}", output);
    let contents = String::from_utf8(output.stdout).unwrap();
    assert!(!contents.contains("inline"), "{contents}");
    assert!(!contents.contains("doc="), "{contents}");
    assert!(!contents.contains("removed_"), "{contents}");
    for kept in ["method", "value", "nested", "kept_foreign", "must_use"] {
        assert!(contents.contains(kept), "{contents}");
    }
    let output = run(source, &["verify", "--deny-warnings"]);
    assert!(output.status.success(), "{:?}", output);
}

#[test]
fn cfg_expansion_preserves_members_and_inlined_module_conditions() {
    let source = r#"
        #[cfg_attr(nightly, cfg(test))]
        mod unavailable;
        #[cfg_attr(any(nightly, unknown), cfg_attr(nightly, codesnip::entry("sample", inline)))]
        mod sample {
            pub struct S { #[cfg(nightly)] pub value: u8, #[cfg(test)] bad: Missing }
            impl S {
                #[cfg_attr(nightly, cfg(nightly))] pub fn enabled() {}
                #[cfg(test)] pub fn disabled() { missing(); }
            }
            pub fn call() { S::enabled(); let _ = S { value: 0 }; }
        }
        #[cfg(feature = "extra")]
        #[codesnip::entry("sample", inline)]
        mod conditional { pub fn selected() { let _: () = 1; } }
        #[cfg_attr(feature = "extra", cfg(test))]
        #[codesnip::entry("sample", inline)]
        mod conditional_attr { pub fn selected_attr() { let _: () = 2; } }
    "#;
    let output = run(source, &["bundle", "sample"]);
    assert!(output.status.success(), "{:?}", output);
    let contents = String::from_utf8(output.stdout).unwrap();
    for kept in ["enabled", "call", "selected", "selected_attr"] {
        assert!(contents.contains(kept), "{contents}");
    }
    // Each inlined module must retain its own condition, including cfg_attr-generated cfg.
    // Remove the second conditional module to test the first independently.
    let first = source
        .split("#[cfg_attr(feature = \"extra\", cfg(test))]")
        .next()
        .unwrap();
    let output = run(first, &["verify", "--deny-warnings"]);
    assert!(output.status.success(), "{:?}", output);
    let output = run(first, &["verify", "--rustc-arg=--cfg=feature=\"extra\""]);
    assert!(!output.status.success(), "{:?}", output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("E0308"));

    let second = source.replace("let _: () = 1;", "");
    let output = run(&second, &["verify", "--rustc-arg=--cfg=feature=\"extra\""]);
    assert!(output.status.success(), "{:?}", output);
    let output = run(&second, &["verify"]);
    assert!(!output.status.success(), "{:?}", output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("E0308"));
}

#[test]
fn cfg_removes_disabled_statements_from_bundles() {
    let source = r#"
        #[codesnip::entry("sample", inline)]
        mod sample {
            pub fn sample() -> u8 {
                #[cfg(nightly,)]
                macro_rules! kept { ($value:expr) => { $value }; }
                #[cfg(nightly)]
                #[cfg(true)]
                #[cfg_attr(false, cfg(test))]
                let value = 1;
                #[cfg(test)]
                let removed_binding = missing();
                #[cfg(test)]
                let Some(removed_let_else) = missing() else { mod removed_let_else_module; };
                #[cfg(test)]
                fn removed_item() { mod removed_item_module; }
                #[cfg(test)]
                removed_call();
                #[cfg_attr(nightly, cfg(test))]
                { mod removed_block_module; }
                #[cfg(test)]
                if true { mod removed_if_module; }
                #[cfg(test)]
                removed_braces! { let ghost count = 0int; }
                #[cfg(test)]
                removed_parens!();
                #[cfg(test)]
                removed_brackets![];
                #[cfg(false,)]
                removed_literal();
                #[cfg_attr(true, cfg(false))]
                removed_literal_attr();
                #[cfg(not(nightly,))]
                removed_trailing_comma();
                #[cfg(feature = "extra")]
                let _: () = 1;
                let nested = || {
                    #[cfg_attr(not(test), cfg(any()))]
                    removed_nested! {}
                    #[cfg(test)]
                    let removed_nested_binding = missing();
                    kept!(value)
                };
                nested()
            }
            pub fn tail_macro() {
                #[cfg(test)]
                removed_tail!()
            }
            pub fn tail_block() -> u8 {
                #[cfg(nightly)]
                { let kept_tail = 1; kept_tail }
                #[cfg(test)]
                { 2 }
            }
        }
    "#;
    let output = run(source, &["bundle", "sample"]);
    assert!(output.status.success(), "{:?}", output);
    let contents = String::from_utf8(output.stdout).unwrap();
    assert!(!contents.contains("removed_"), "{contents}");
    assert!(!contents.contains("cfg(any())"), "{contents}");
    assert!(!contents.contains("cfg(true)"), "{contents}");
    assert!(!contents.contains("cfg_attr(false"), "{contents}");
    for kept in [
        "kept",
        "value",
        "nested",
        "tail_macro",
        "tail_block",
        "extra",
    ] {
        assert!(contents.contains(kept), "{contents}");
    }
    let output = run(source, &["verify", "--deny-warnings"]);
    assert!(output.status.success(), "{:?}", output);
    let output = run(source, &["verify", "--rustc-arg=--cfg=feature=\"extra\""]);
    assert!(!output.status.success(), "{:?}", output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("E0308"));
}

#[test]
fn cfg_resolves_known_parts_of_unknown_conditions() {
    for (attribute, active_with_extra) in [
        (r#"cfg(all(nightly, feature = "extra"))"#, true),
        (r#"cfg(all(nightly, feature = "extra"),)"#, true),
        (r#"cfg(any(test, feature = "extra"))"#, true),
        (r#"cfg(not(not(all(nightly, feature = "extra"),),))"#, true),
        (
            r#"cfg(any(false, all(true, nightly, feature = "extra")))"#,
            true,
        ),
        (
            r#"cfg_attr(true, cfg(all(nightly, feature = "extra")))"#,
            true,
        ),
        (
            r#"cfg(not(any(test, not(all(nightly, feature = "extra")))))"#,
            true,
        ),
        (
            r#"cfg_attr(all(nightly, feature = "extra"), cfg(test))"#,
            false,
        ),
        (r#"cfg_attr(feature = "extra", cfg(test,))"#, false),
        (
            r#"cfg_attr(feature = "extra", cfg_attr(nightly, cfg(any(test, not(nightly)))))"#,
            false,
        ),
    ] {
        let source =
            format!("#[codesnip::entry] pub fn sample() {{ #[{attribute}] let _: () = 1; }}");
        for extra in [false, true] {
            // The source config must win even when rustc enables `test`.
            let mut args = vec!["verify", "--rustc-arg=--cfg=test"];
            if extra {
                args.push("--rustc-arg=--cfg=feature=\"extra\"");
            }
            let output = run(&source, &args);
            assert_eq!(
                output.status.success(),
                extra != active_with_extra,
                "{attribute}, extra={extra}: {output:?}"
            );
            if !output.status.success() {
                assert!(String::from_utf8_lossy(&output.stderr).contains("E0308"));
            }
        }
    }

    let source = r#"
        #[codesnip::entry]
        pub fn sample() {
            #[cfg_attr(all(nightly, feature = "extra"), allow(unused_variables))]
            let unused = 1;
        }
    "#;
    let output = run(source, &["verify", "--deny-warnings"]);
    assert!(!output.status.success(), "{:?}", output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("unused_variables"));
    let output = run(
        source,
        &[
            "verify",
            "--deny-warnings",
            "--rustc-arg=--cfg=feature=\"extra\"",
        ],
    );
    assert!(output.status.success(), "{:?}", output);
}

#[test]
fn cfg_removes_disabled_members_before_visiting_children() {
    let source = r#"
        #[codesnip::entry("sample", inline)]
        mod sample {
            pub struct S;
            impl S {
                #[cfg(nightly)] pub fn kept() {}
                #[cfg(test)] pub fn removed_method() { mod removed_module; }
                #[cfg_attr(nightly, cfg(test))]
                pub const REMOVED: u8 = { mod removed_module; 0 };
                #[cfg(test)] type Removed = [u8; { mod removed_module; 0 }];
                #[cfg(test)] removed_macro! { let ghost count = 0int; }
            }
            pub trait T {
                fn kept() {}
                #[cfg(test)] fn removed_method() { mod removed_module; }
                #[cfg(test)] const REMOVED: u8 = { mod removed_module; 0 };
                #[cfg(test)] type Removed = [u8; { mod removed_module; 0 }];
                #[cfg(test)] removed_macro!();
            }
            unsafe extern "C" {
                pub fn kept_foreign();
                #[cfg(test)] fn removed_foreign(_: [u8; { mod removed_module; 0 }]);
                #[cfg(test)] static REMOVED: [u8; { mod removed_module; 0 }];
                #[cfg(test)] type Removed;
                #[cfg(test)] removed_macro!();
            }
            pub fn sample() { S::kept(); }
            impl T for S {
                #[cfg(feature = "extra")] fn kept() { let _: () = 1; }
            }
        }
    "#;
    let output = run(source, &["bundle", "sample"]);
    assert!(output.status.success(), "{output:?}");
    let contents = String::from_utf8(output.stdout).unwrap();
    assert!(!contents.to_lowercase().contains("removed"), "{contents}");
    assert!(!contents.contains("cfg(any())"), "{contents}");
    assert!(contents.contains("kept_foreign"), "{contents}");
    let output = run(source, &["verify", "--deny-warnings"]);
    assert!(output.status.success(), "{output:?}");
    let output = run(source, &["verify", "--rustc-arg=--cfg=feature=\"extra\""]);
    assert!(!output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("E0308"));
}

#[test]
fn cfg_removes_disabled_collection_elements_before_visiting_children() {
    let source = r#"
        #[codesnip::entry("sample", inline)]
        mod sample {
            pub struct Named {
                #[cfg(test)] pub removed: [u8; { mod removed_module; 0 }],
                #[cfg(nightly)] pub kept: u8,
            }
            pub struct Tuple(#[cfg(test)] [u8; { mod removed_module; 0 }], pub u8);
            pub union Union {
                #[cfg(test)] removed: [u8; { mod removed_module; 0 }],
                pub kept: u8,
            }
            pub enum E {
                #[cfg(test)] Removed = { mod removed_module; 0 },
                Kept,
            }
            pub struct Generic<
                #[cfg(test)] 'removed,
                #[cfg(test)] Removed = [u8; { mod removed_module; 0 }],
                #[cfg(test)] const REMOVED: usize = { mod removed_module; 0 },
            >;
            pub fn argument(#[cfg(test)] _: [u8; { mod removed_module; 0 }], kept: u8) -> u8 {
                kept
            }
            impl Named {
                pub fn method(&self, #[cfg(test)] _: [u8; { mod removed_module; 0 }], kept: u8) -> u8 {
                    self.kept + kept
                }
                pub fn associated(#[cfg(test)] &self) {}
            }
            pub type Pointer = for<#[cfg(test)] 'removed> extern "C" fn(
                #[cfg(test)] [u8; { mod removed_module; 0 }],
                #[cfg(test)] ...
            );
            unsafe extern "C" {
                pub fn variadic(#[cfg(test)] _: [u8; { mod removed_module; 0 }], #[cfg(test)] ...);
            }
            pub fn sample() {
                let _: [u8; 1] = [1, #[cfg(feature = "extra")] 2];
                let _: [u8; 1] = [#[cfg(test)] { mod removed_module; 0 }, 1];
                let _: (u8,) = (#[cfg(test)] { mod removed_module; 0 }, 1);
                let _: (u8,) = (1, #[cfg(test)] { mod removed_module; 0 });
                let _: () = (#[cfg(test)] { mod removed_module; 0 },);
                let _: [u8; 0] = [#[cfg(test)] { mod removed_module; 0 }];
                let value = Named { #[cfg(test)] removed: { mod removed_module; 0 }, kept: 1 };
                let Named { #[cfg(test)] removed, kept } = value;
                let _: u8 = argument(#[cfg(test)] { mod removed_module; 0 }, kept);
                let _: u8 = value.method(#[cfg(test)] { mod removed_module; 0 }, kept);
                let _: Tuple = Tuple(#[cfg(test)] { mod removed_module; 0 }, kept);
                let _: Generic = Generic;
                Named::associated();
                let closure = |#[cfg(test)] removed: [u8; { mod removed_module; 0 }], kept: u8| kept;
                let _: u8 = closure(kept);
                let closure = |#[cfg(test)] removed, kept| kept;
                let _: u8 = closure(kept);
                let _: u8 = match E::Kept {
                    #[cfg_attr(nightly, cfg(test))]
                    E::Removed => { mod removed_module; 0 },
                    E::Kept => kept,
                };
            }
        }
    "#;
    let output = run(source, &["bundle", "sample"]);
    assert!(output.status.success(), "{output:?}");
    let contents = String::from_utf8(output.stdout).unwrap();
    assert!(!contents.to_lowercase().contains("removed"), "{contents}");
    assert!(!contents.contains("cfg(any())"), "{contents}");
    let output = run(source, &["verify", "--deny-warnings"]);
    assert!(output.status.success(), "{output:?}");
    let output = run(source, &["verify", "--rustc-arg=--cfg=feature=\"extra\""]);
    assert!(!output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("E0308"));
}

#[test]
fn cfg_matches_raw_and_plain_identifiers() {
    let source = r##"
        #[codesnip::entry("sample", inline)]
        mod sample {
            #[r#cfg(r#nightly)]
            pub fn kept() {
                #[r#cfg(r#test)] mod removed_module;
                #[r#cfg_attr(r#nightly, r#cfg_attr(r#not(r#test), r#cfg(r#test)))]
                mod removed_nested;
                #[cfg(r#all(r#nightly, r#any(r#test, r#feature = "extra")))]
                let _: () = 1;
                #[cfg(r#any(r#test, r#not(r#nightly)))] mod removed_operator;
            }
            #[cfg(nightly)] pub fn also_kept() {}
            #[cfg(test)] mod removed_plain;
            #[cfg(r#feature = "enabled")] pub fn feature_kept() {}
            #[cfg(feature = "enabled")] pub fn feature_also_kept() {}
            #[cfg(feature = r"enabled")] pub fn raw_string_kept() {}
            #[cfg(r#feature = "\x65nabled")] pub fn escaped_string_kept() {}
            #[cfg(r#feature = "disabled")] mod removed_feature;
            #[cfg(feature = "disabled")] mod removed_plain_feature;
            #[cfg(feature = r"disabled")] mod removed_raw_string;
            #[cfg(r#feature = "dis\u{61}bled")] mod removed_escaped_string;
            #[cfg(feature = "r#extra")] mod removed_value;
            #[cfg(r#false)] pub fn literal_name_kept() {}
            #[cfg(r#true)] mod removed_literal_name;
            pub fn sample() {
                kept(); also_kept(); feature_kept(); feature_also_kept(); literal_name_kept();
                raw_string_kept(); escaped_string_kept();
            }
        }
    "##;
    for (enabled, disabled, key, enabled_value, disabled_value) in [
        ("nightly", "test", "feature", "\"enabled\"", "\"disabled\""),
        (
            "r#nightly",
            "r#test",
            "r#feature",
            "r\"enabled\"",
            "\"dis\\u{61}bled\"",
        ),
    ] {
        let config = serde_json::json!({
            "cfg_enable": [enabled, format!("{key} = {enabled_value}"), "r#false"],
            "cfg_disable": [disabled, format!("{key} = {disabled_value}"), "feature = \"r#extra\"", "r#true"]
        });
        let output = run_with_config(source, &["bundle", "sample"], config.clone());
        assert!(output.status.success(), "{output:?}");
        let contents = String::from_utf8(output.stdout).unwrap();
        assert!(!contents.contains("removed"), "{contents}");
        let output = run_with_config(source, &["verify", "--deny-warnings"], config.clone());
        assert!(output.status.success(), "{output:?}");
        let output = run_with_config(
            source,
            &["verify", "--rustc-arg=--cfg=feature=\"extra\""],
            config.clone(),
        );
        assert!(!output.status.success(), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("E0308"));

        let conditional = r#"
            #[codesnip::entry("sample", inline)]
            #[r#cfg_attr(r#feature = "extra", r#cfg(r#test))]
            mod conditional {
                pub fn conditional() { let _: () = 1; }
            }
        "#;
        for extra in [false, true] {
            let mut args = vec!["verify"];
            if extra {
                args.push("--rustc-arg=--cfg=feature=\"extra\"");
            }
            let output = run_with_config(conditional, &args, config.clone());
            assert_eq!(output.status.success(), extra, "{output:?}");
            if !extra {
                assert!(String::from_utf8_lossy(&output.stderr).contains("E0308"));
            }
        }
    }
}

#[test]
fn cfg_skips_disabled_files_and_external_module_contents() {
    let source = "#![cfg(test)] mod removed_module; #[codesnip::entry] pub fn removed() {}";
    let output = run(source, &["list"]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let output = run(
        "#![cfg(nightly)] #[codesnip::entry] pub fn kept() {}",
        &["list"],
    );
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("kept"));

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(
        &path,
        r#"
            #[codesnip::entry("sample", inline)]
            #[r#cfg_attr(nightly, r#path = "external.rs")]
            mod renamed;
            #[codesnip::entry("sample")] pub fn kept() {}
        "#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("external.rs"),
        "#![cfg_attr(nightly, cfg(test))] mod removed_module; pub fn removed() {}",
    )
    .unwrap();
    let config = serde_json::json!({ "sources": [{ "path": path }] });
    let output = run_with_config("", &["bundle", "sample"], config.clone());
    assert!(output.status.success(), "{output:?}");
    let contents = String::from_utf8(output.stdout).unwrap();
    assert!(contents.contains("kept"), "{contents}");
    assert!(!contents.contains("removed"), "{contents}");
    let output = run_with_config("", &["verify", "--deny-warnings"], config);
    assert!(output.status.success(), "{output:?}");

    let config = serde_json::json!({
        "sources": [{ "path": path }], "cfg_enable": ["nightly", "test"], "cfg_disable": []
    });
    let output = run_with_config("", &["bundle", "sample"], config);
    assert!(!output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("removed_module"));
}

#[test]
fn cfg_preserves_conditions_of_files_and_parent_modules() {
    for (prefix, active_with_extra) in [
        ("#![cfg(all(nightly, feature = \"extra\"))]", true),
        ("#[cfg_attr(feature = \"extra\", cfg(test))]", false),
    ] {
        let source = format!(
            "{prefix} mod outer {{ mod inner {{ #[codesnip::entry] pub fn sample() {{ let _: () = 1; }} }} }}"
        );
        for extra in [false, true] {
            let mut args = vec!["verify"];
            if extra {
                args.push("--rustc-arg=--cfg=feature=\"extra\"");
            }
            let output = run(&source, &args);
            assert_eq!(
                output.status.success(),
                extra != active_with_extra,
                "{output:?}"
            );
            if !output.status.success() {
                assert!(String::from_utf8_lossy(&output.stderr).contains("E0308"));
            }
        }
    }
}

#[test]
fn and_entries_are_bundled_and_verified() {
    let source = r#"
        #[codesnip::entry] pub struct A;
        #[codesnip::entry] pub trait B {}
        #[codesnip::entry(when("A", "B"))] impl B for A {}
        #[codesnip::entry("C", when("A", "B"))]
        pub fn c() { fn check<T: B>() {} check::<A>(); }
    "#;
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache.bin");
    let output = run(source, &["cache", cache.to_str().unwrap()]);
    assert!(output.status.success(), "{:?}", output);
    let cache_arg = format!("--use-cache={}", cache.display());
    for (args, has_impl) in [
        (vec!["bundle", "A"], false),
        (vec!["bundle", "B"], false),
        (vec!["bundle", "A", "B"], true),
        (vec!["bundle", "C"], true),
        (vec!["bundle", "B", "--excludes", "A"], true),
    ] {
        let mut command = vec![cache_arg.as_str()];
        command.extend(args);
        let output = run("", &command);
        assert!(output.status.success(), "{:?}", output);
        let contents = String::from_utf8(output.stdout).unwrap();
        assert_eq!(contents.contains("impl B for A"), has_impl, "{contents}");
    }
    let output = run("", &[&cache_arg, "verify", "--deny-warnings"]);
    assert!(output.status.success(), "{:?}", output);
    std::fs::write(&cache, b"old cache").unwrap();
    let output = run("", &[&cache_arg, "list"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported cache format"));
}
