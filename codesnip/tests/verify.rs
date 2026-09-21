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
