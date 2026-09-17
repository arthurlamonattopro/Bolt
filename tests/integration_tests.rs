use assert_cmd::Command as AssertCmd;
use flate2::Compression;
use flate2::write::GzEncoder;
use serde_json::json;
use std::fs;
use std::process::Command;
use tar::Builder;
use tempfile::tempdir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Helper to create a gzipped tarball with npm layout (`package/package.json`, `package/index.js`, etc.)
fn create_test_tarball(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut tar = Builder::new(&mut enc);
        for &(file_path, file_data) in files {
            let full_path = format!("package/{}", file_path);
            let mut header = tar::Header::new_gnu();
            header.set_size(file_data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, full_path, file_data).unwrap();
        }
        tar.finish().unwrap();
    }
    enc.finish().unwrap()
}

fn compute_sha512_integrity(bytes: &[u8]) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha512};
    let mut hasher = Sha512::new();
    hasher.update(bytes);
    let hash = base64::engine::general_purpose::STANDARD.encode(hasher.finalize());
    format!("sha512-{}", hash)
}

#[tokio::test]
async fn test_install_and_node_require() {
    let mock_server = MockServer::start().await;

    // Create a mock package "is-even@1.0.0" that depends on "is-odd@^1.0.0"
    let is_even_tarball = create_test_tarball(&[
        (
            "package.json",
            br#"{"name":"is-even","version":"1.0.0","main":"index.js","dependencies":{"is-odd":"^1.0.0"}}"#,
        ),
        (
            "index.js",
            br#"const isOdd = require('is-odd'); module.exports = function isEven(n) { return !isOdd(n); };"#,
        ),
    ]);
    let is_even_integrity = compute_sha512_integrity(&is_even_tarball);

    // Create a mock package "is-odd@1.0.0"
    let is_odd_tarball = create_test_tarball(&[
        (
            "package.json",
            br#"{"name":"is-odd","version":"1.0.0","main":"index.js"}"#,
        ),
        (
            "index.js",
            br#"module.exports = function isOdd(n) { return (n % 2) !== 0; };"#,
        ),
    ]);
    let is_odd_integrity = compute_sha512_integrity(&is_odd_tarball);

    let is_even_meta = json!({
        "name": "is-even",
        "dist-tags": { "latest": "1.0.0" },
        "versions": {
            "1.0.0": {
                "name": "is-even",
                "version": "1.0.0",
                "dist": {
                    "tarball": format!("{}/tarballs/is-even-1.0.0.tgz", mock_server.uri()),
                    "integrity": is_even_integrity
                },
                "dependencies": {
                    "is-odd": "^1.0.0"
                }
            }
        }
    });

    let is_odd_meta = json!({
        "name": "is-odd",
        "dist-tags": { "latest": "1.0.0" },
        "versions": {
            "1.0.0": {
                "name": "is-odd",
                "version": "1.0.0",
                "dist": {
                    "tarball": format!("{}/tarballs/is-odd-1.0.0.tgz", mock_server.uri()),
                    "integrity": is_odd_integrity
                }
            }
        }
    });

    Mock::given(method("GET"))
        .and(path("/is-even"))
        .respond_with(ResponseTemplate::new(200).set_body_json(is_even_meta))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/tarballs/is-even-1.0.0.tgz"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(is_even_tarball, "application/octet-stream"),
        )
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/is-odd"))
        .respond_with(ResponseTemplate::new(200).set_body_json(is_odd_meta))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/tarballs/is-odd-1.0.0.tgz"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(is_odd_tarball, "application/octet-stream"),
        )
        .mount(&mock_server)
        .await;

    let temp_proj = tempdir().unwrap();
    let proj_path = temp_proj.path();

    let mut cmd = AssertCmd::cargo_bin("bolt").unwrap();
    cmd.current_dir(proj_path)
        .arg("--registry")
        .arg(mock_server.uri())
        .arg("install")
        .arg("is-even");

    cmd.assert().success();

    assert!(
        proj_path
            .join("node_modules")
            .join("is-even")
            .join("index.js")
            .exists()
    );
    assert!(
        proj_path
            .join("node_modules")
            .join("is-odd")
            .join("index.js")
            .exists()
    );

    let lock_content = fs::read_to_string(proj_path.join("package-lock.json")).unwrap();
    let lock_json: serde_json::Value = serde_json::from_str(&lock_content).unwrap();
    assert_eq!(lock_json["lockfileVersion"], 3);
    assert!(lock_json["packages"]["node_modules/is-even"].is_object());
    assert!(lock_json["packages"]["node_modules/is-odd"].is_object());

    let test_script = r#"
        const isEven = require('is-even');
        if (!isEven(4) || isEven(5)) {
            process.exit(1);
        }
        console.log("SUCCESS");
    "#;

    let node_status = Command::new("node")
        .current_dir(proj_path)
        .arg("-e")
        .arg(test_script)
        .status()
        .expect("Failed to execute node");

    assert!(node_status.success());
}

#[tokio::test]
async fn test_bin_shim_and_run_script() {
    let mock_server = MockServer::start().await;

    let my_cli_tarball = create_test_tarball(&[
        (
            "package.json",
            br#"{"name":"my-cli","version":"1.0.0","bin":{"my-cli":"bin/cli.js"}}"#,
        ),
        (
            "bin/cli.js",
            br#"#!/usr/bin/env node
console.log("HELLO_FROM_MY_CLI", process.argv.slice(2).join(" "));
"#,
        ),
    ]);
    let my_cli_integrity = compute_sha512_integrity(&my_cli_tarball);

    let my_cli_meta = json!({
        "name": "my-cli",
        "dist-tags": { "latest": "1.0.0" },
        "versions": {
            "1.0.0": {
                "name": "my-cli",
                "version": "1.0.0",
                "bin": {
                    "my-cli": "bin/cli.js"
                },
                "dist": {
                    "tarball": format!("{}/tarballs/my-cli-1.0.0.tgz", mock_server.uri()),
                    "integrity": my_cli_integrity
                }
            }
        }
    });

    Mock::given(method("GET"))
        .and(path("/my-cli"))
        .respond_with(ResponseTemplate::new(200).set_body_json(my_cli_meta))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/tarballs/my-cli-1.0.0.tgz"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(my_cli_tarball, "application/octet-stream"),
        )
        .mount(&mock_server)
        .await;

    let temp_proj = tempdir().unwrap();
    let proj_path = temp_proj.path();

    let initial_pkg = json!({
        "name": "test-app",
        "version": "1.0.0",
        "scripts": {
            "test-cli": "my-cli arg1 arg2"
        },
        "dependencies": {
            "my-cli": "^1.0.0"
        }
    });
    fs::write(proj_path.join("package.json"), initial_pkg.to_string()).unwrap();

    let mut cmd = AssertCmd::cargo_bin("bolt").unwrap();
    cmd.current_dir(proj_path)
        .arg("--registry")
        .arg(mock_server.uri())
        .arg("install");
    cmd.assert().success();

    let bin_dir = proj_path.join("node_modules").join(".bin");
    #[cfg(windows)]
    assert!(bin_dir.join("my-cli.cmd").exists());
    #[cfg(not(windows))]
    assert!(bin_dir.join("my-cli").exists());

    let mut run_cmd = AssertCmd::cargo_bin("bolt").unwrap();
    run_cmd.current_dir(proj_path).arg("run").arg("test-cli");
    let output = run_cmd.assert().success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(stdout.contains("HELLO_FROM_MY_CLI arg1 arg2"));
}

#[tokio::test]
async fn test_ci_clean_install() {
    let mock_server = MockServer::start().await;

    let dummy_tarball = create_test_tarball(&[
        ("package.json", br#"{"name":"dummy","version":"1.0.0"}"#),
        ("index.js", br#"module.exports = 'dummy';"#),
    ]);
    let dummy_integrity = compute_sha512_integrity(&dummy_tarball);

    let dummy_meta = json!({
        "name": "dummy",
        "dist-tags": { "latest": "1.0.0" },
        "versions": {
            "1.0.0": {
                "name": "dummy",
                "version": "1.0.0",
                "dist": {
                    "tarball": format!("{}/tarballs/dummy-1.0.0.tgz", mock_server.uri()),
                    "integrity": dummy_integrity
                }
            }
        }
    });

    Mock::given(method("GET"))
        .and(path("/dummy"))
        .respond_with(ResponseTemplate::new(200).set_body_json(dummy_meta))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/tarballs/dummy-1.0.0.tgz"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(dummy_tarball, "application/octet-stream"),
        )
        .mount(&mock_server)
        .await;

    let temp_proj = tempdir().unwrap();
    let proj_path = temp_proj.path();

    let mut cmd = AssertCmd::cargo_bin("bolt").unwrap();
    cmd.current_dir(proj_path)
        .arg("--registry")
        .arg(mock_server.uri())
        .arg("install")
        .arg("dummy");
    cmd.assert().success();

    fs::remove_dir_all(proj_path.join("node_modules")).unwrap();
    assert!(!proj_path.join("node_modules").exists());

    let mut ci_cmd = AssertCmd::cargo_bin("bolt").unwrap();
    ci_cmd
        .current_dir(proj_path)
        .arg("--registry")
        .arg(mock_server.uri())
        .arg("ci");
    ci_cmd.assert().success();

    assert!(
        proj_path
            .join("node_modules")
            .join("dummy")
            .join("index.js")
            .exists()
    );
}

#[tokio::test]
async fn test_nested_dependency_conflict_resolution() {
    let mock_server = MockServer::start().await;

    // Package A requires B@^1.0.0
    // Package C requires B@^2.0.0
    // Top-level requires A and C.
    // One B should be at top-level `node_modules/B`, and the other nested under `node_modules/C/node_modules/B`.
    let b1_tarball = create_test_tarball(&[
        ("package.json", br#"{"name":"pkg-b","version":"1.0.0"}"#),
        ("index.js", br#"module.exports = 'B1';"#),
    ]);
    let b1_integrity = compute_sha512_integrity(&b1_tarball);

    let b2_tarball = create_test_tarball(&[
        ("package.json", br#"{"name":"pkg-b","version":"2.0.0"}"#),
        ("index.js", br#"module.exports = 'B2';"#),
    ]);
    let b2_integrity = compute_sha512_integrity(&b2_tarball);

    let a_tarball = create_test_tarball(&[
        (
            "package.json",
            br#"{"name":"pkg-a","version":"1.0.0","dependencies":{"pkg-b":"^1.0.0"}}"#,
        ),
        (
            "index.js",
            br#"const b = require('pkg-b'); module.exports = 'A_' + b;"#,
        ),
    ]);
    let a_integrity = compute_sha512_integrity(&a_tarball);

    let c_tarball = create_test_tarball(&[
        (
            "package.json",
            br#"{"name":"pkg-c","version":"1.0.0","dependencies":{"pkg-b":"^2.0.0"}}"#,
        ),
        (
            "index.js",
            br#"const b = require('pkg-b'); module.exports = 'C_' + b;"#,
        ),
    ]);
    let c_integrity = compute_sha512_integrity(&c_tarball);

    let b_meta = json!({
        "name": "pkg-b",
        "dist-tags": { "latest": "2.0.0" },
        "versions": {
            "1.0.0": {
                "name": "pkg-b",
                "version": "1.0.0",
                "dist": {
                    "tarball": format!("{}/tarballs/pkg-b-1.0.0.tgz", mock_server.uri()),
                    "integrity": b1_integrity
                }
            },
            "2.0.0": {
                "name": "pkg-b",
                "version": "2.0.0",
                "dist": {
                    "tarball": format!("{}/tarballs/pkg-b-2.0.0.tgz", mock_server.uri()),
                    "integrity": b2_integrity
                }
            }
        }
    });

    let a_meta = json!({
        "name": "pkg-a",
        "dist-tags": { "latest": "1.0.0" },
        "versions": {
            "1.0.0": {
                "name": "pkg-a",
                "version": "1.0.0",
                "dist": {
                    "tarball": format!("{}/tarballs/pkg-a-1.0.0.tgz", mock_server.uri()),
                    "integrity": a_integrity
                },
                "dependencies": {
                    "pkg-b": "^1.0.0"
                }
            }
        }
    });

    let c_meta = json!({
        "name": "pkg-c",
        "dist-tags": { "latest": "1.0.0" },
        "versions": {
            "1.0.0": {
                "name": "pkg-c",
                "version": "1.0.0",
                "dist": {
                    "tarball": format!("{}/tarballs/pkg-c-1.0.0.tgz", mock_server.uri()),
                    "integrity": c_integrity
                },
                "dependencies": {
                    "pkg-b": "^2.0.0"
                }
            }
        }
    });

    Mock::given(method("GET"))
        .and(path("/pkg-b"))
        .respond_with(ResponseTemplate::new(200).set_body_json(b_meta))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tarballs/pkg-b-1.0.0.tgz"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(b1_tarball, "application/octet-stream"),
        )
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tarballs/pkg-b-2.0.0.tgz"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(b2_tarball, "application/octet-stream"),
        )
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/pkg-a"))
        .respond_with(ResponseTemplate::new(200).set_body_json(a_meta))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tarballs/pkg-a-1.0.0.tgz"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(a_tarball, "application/octet-stream"),
        )
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/pkg-c"))
        .respond_with(ResponseTemplate::new(200).set_body_json(c_meta))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tarballs/pkg-c-1.0.0.tgz"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(c_tarball, "application/octet-stream"),
        )
        .mount(&mock_server)
        .await;

    let temp_proj = tempdir().unwrap();
    let proj_path = temp_proj.path();

    let initial_pkg = json!({
        "name": "conflict-app",
        "version": "1.0.0",
        "dependencies": {
            "pkg-a": "^1.0.0",
            "pkg-c": "^1.0.0"
        }
    });
    fs::write(proj_path.join("package.json"), initial_pkg.to_string()).unwrap();

    let mut cmd = AssertCmd::cargo_bin("bolt").unwrap();
    cmd.current_dir(proj_path)
        .arg("--registry")
        .arg(mock_server.uri())
        .arg("install");
    cmd.assert().success();

    // Verify both versions exist: one at top-level node_modules/pkg-b, one nested at node_modules/pkg-c/node_modules/pkg-b
    assert!(proj_path.join("node_modules").join("pkg-b").exists());
    assert!(
        proj_path
            .join("node_modules")
            .join("pkg-c")
            .join("node_modules")
            .join("pkg-b")
            .exists()
    );

    // Test requiring both via Node.js: Node should correctly resolve the respective versions
    let test_script = r#"
        const a = require('pkg-a');
        const c = require('pkg-c');
        console.log(a, c);
        if (a !== 'A_B1' || c !== 'C_B2') {
            process.exit(1);
        }
    "#;

    let node_status = Command::new("node")
        .current_dir(proj_path)
        .arg("-e")
        .arg(test_script)
        .status()
        .expect("Failed to execute node");

    assert!(node_status.success());
}

#[tokio::test]
async fn test_dependency_cycle_handling() {
    let mock_server = MockServer::start().await;

    // Cycle: circle-a -> circle-b -> circle-a
    let ca_tarball = create_test_tarball(&[
        (
            "package.json",
            br#"{"name":"circle-a","version":"1.0.0","dependencies":{"circle-b":"^1.0.0"}}"#,
        ),
        ("index.js", br#"module.exports = 'CA';"#),
    ]);
    let ca_integrity = compute_sha512_integrity(&ca_tarball);

    let cb_tarball = create_test_tarball(&[
        (
            "package.json",
            br#"{"name":"circle-b","version":"1.0.0","dependencies":{"circle-a":"^1.0.0"}}"#,
        ),
        ("index.js", br#"module.exports = 'CB';"#),
    ]);
    let cb_integrity = compute_sha512_integrity(&cb_tarball);

    let ca_meta = json!({
        "name": "circle-a",
        "dist-tags": { "latest": "1.0.0" },
        "versions": {
            "1.0.0": {
                "name": "circle-a",
                "version": "1.0.0",
                "dist": {
                    "tarball": format!("{}/tarballs/circle-a-1.0.0.tgz", mock_server.uri()),
                    "integrity": ca_integrity
                },
                "dependencies": {
                    "circle-b": "^1.0.0"
                }
            }
        }
    });

    let cb_meta = json!({
        "name": "circle-b",
        "dist-tags": { "latest": "1.0.0" },
        "versions": {
            "1.0.0": {
                "name": "circle-b",
                "version": "1.0.0",
                "dist": {
                    "tarball": format!("{}/tarballs/circle-b-1.0.0.tgz", mock_server.uri()),
                    "integrity": cb_integrity
                },
                "dependencies": {
                    "circle-a": "^1.0.0"
                }
            }
        }
    });

    Mock::given(method("GET"))
        .and(path("/circle-a"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ca_meta))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tarballs/circle-a-1.0.0.tgz"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(ca_tarball, "application/octet-stream"),
        )
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/circle-b"))
        .respond_with(ResponseTemplate::new(200).set_body_json(cb_meta))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tarballs/circle-b-1.0.0.tgz"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(cb_tarball, "application/octet-stream"),
        )
        .mount(&mock_server)
        .await;

    let temp_proj = tempdir().unwrap();
    let proj_path = temp_proj.path();

    let mut cmd = AssertCmd::cargo_bin("bolt").unwrap();
    cmd.current_dir(proj_path)
        .arg("--registry")
        .arg(mock_server.uri())
        .arg("install")
        .arg("circle-a");

    // Must resolve cleanly without infinite recursion / stack overflow
    cmd.assert().success();

    assert!(proj_path.join("node_modules").join("circle-a").exists());
    assert!(proj_path.join("node_modules").join("circle-b").exists());
}

#[tokio::test]
async fn test_npm_alias_dependency_resolution() {
    let mock_server = MockServer::start().await;

    // Real package "original-pkg"
    let orig_tarball = create_test_tarball(&[
        (
            "package.json",
            br#"{"name":"original-pkg","version":"1.2.3"}"#,
        ),
        ("index.js", br#"module.exports = 'ORIGINAL_1.2.3';"#),
    ]);
    let orig_integrity = compute_sha512_integrity(&orig_tarball);

    let orig_meta = json!({
        "name": "original-pkg",
        "dist-tags": { "latest": "1.2.3" },
        "versions": {
            "1.2.3": {
                "name": "original-pkg",
                "version": "1.2.3",
                "dist": {
                    "tarball": format!("{}/tarballs/original-pkg-1.2.3.tgz", mock_server.uri()),
                    "integrity": orig_integrity
                }
            }
        }
    });

    Mock::given(method("GET"))
        .and(path("/original-pkg"))
        .respond_with(ResponseTemplate::new(200).set_body_json(orig_meta))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tarballs/original-pkg-1.2.3.tgz"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(orig_tarball, "application/octet-stream"),
        )
        .mount(&mock_server)
        .await;

    let temp_proj = tempdir().unwrap();
    let proj_path = temp_proj.path();

    // App uses alias: "custom-alias": "npm:original-pkg@^1.0.0"
    let initial_pkg = json!({
        "name": "alias-app",
        "version": "1.0.0",
        "dependencies": {
            "custom-alias": "npm:original-pkg@^1.0.0"
        }
    });
    fs::write(proj_path.join("package.json"), initial_pkg.to_string()).unwrap();

    let mut cmd = AssertCmd::cargo_bin("bolt").unwrap();
    cmd.current_dir(proj_path)
        .arg("--registry")
        .arg(mock_server.uri())
        .arg("install");
    cmd.assert().success();

    // Installed folder in node_modules must be `custom-alias`
    assert!(
        proj_path
            .join("node_modules")
            .join("custom-alias")
            .join("index.js")
            .exists()
    );

    let test_script = r#"
        const a = require('custom-alias');
        if (a !== 'ORIGINAL_1.2.3') {
            process.exit(1);
        }
        console.log("ALIAS_SUCCESS");
    "#;

    let node_status = Command::new("node")
        .current_dir(proj_path)
        .arg("-e")
        .arg(test_script)
        .status()
        .expect("Failed to execute node");

    assert!(node_status.success());
}

#[tokio::test]
async fn test_monorepo_workspaces_linking() {
    let temp_proj = tempdir().unwrap();
    let proj_path = temp_proj.path();

    let pkg_a_dir = proj_path.join("packages").join("pkg-a");
    let pkg_b_dir = proj_path.join("packages").join("pkg-b");
    fs::create_dir_all(&pkg_a_dir).unwrap();
    fs::create_dir_all(&pkg_b_dir).unwrap();

    let root_pkg = json!({
        "name": "root-monorepo",
        "version": "1.0.0",
        "workspaces": [
            "packages/*"
        ]
    });
    fs::write(proj_path.join("package.json"), root_pkg.to_string()).unwrap();

    let pkg_a_json = json!({
        "name": "pkg-a",
        "version": "1.0.0",
        "main": "index.js"
    });
    fs::write(pkg_a_dir.join("package.json"), pkg_a_json.to_string()).unwrap();
    fs::write(pkg_a_dir.join("index.js"), "module.exports = 'I_AM_PKG_A';").unwrap();

    let pkg_b_json = json!({
        "name": "pkg-b",
        "version": "1.0.0",
        "main": "index.js"
    });
    fs::write(pkg_b_dir.join("package.json"), pkg_b_json.to_string()).unwrap();
    fs::write(
        pkg_b_dir.join("index.js"),
        "const a = require('pkg-a'); module.exports = 'PKG_B_SEES_' + a;",
    )
    .unwrap();

    let mut cmd = AssertCmd::cargo_bin("bolt").unwrap();
    cmd.current_dir(proj_path).arg("install");
    cmd.assert().success();

    assert!(proj_path.join("node_modules").join("pkg-a").exists());
    assert!(proj_path.join("node_modules").join("pkg-b").exists());

    let test_script = r#"
        const b = require('pkg-b');
        if (b !== 'PKG_B_SEES_I_AM_PKG_A') {
            process.exit(1);
        }
        console.log("MONOREPO_SUCCESS");
    "#;

    let node_status = Command::new("node")
        .current_dir(proj_path)
        .arg("-e")
        .arg(test_script)
        .status()
        .expect("Failed to execute node");

    assert!(node_status.success());
}

#[tokio::test]
async fn test_lifecycle_scripts_execution_and_ignore_scripts() {
    let mock_server = MockServer::start().await;

    let lifecycle_tarball = create_test_tarball(&[
        (
            "package.json",
            br#"{"name":"lifecycle-pkg","version":"1.0.0","scripts":{"postinstall":"node index.js"}}"#,
        ),
        ("index.js", br#"const fs = require('fs'); fs.writeFileSync('ran.txt', 'OK');"#),
    ]);
    let lifecycle_integrity = compute_sha512_integrity(&lifecycle_tarball);
    let lifecycle_meta = json!({
        "name": "lifecycle-pkg",
        "dist-tags": { "latest": "1.0.0" },
        "versions": {
            "1.0.0": {
                "name": "lifecycle-pkg",
                "version": "1.0.0",
                "scripts": {
                    "postinstall": "node index.js"
                },
                "dist": {
                    "tarball": format!("{}/tarballs/lifecycle-1.0.0.tgz", mock_server.uri()),
                    "integrity": lifecycle_integrity
                }
            }
        }
    });
    Mock::given(method("GET"))
        .and(path("/lifecycle-pkg"))
        .respond_with(ResponseTemplate::new(200).set_body_json(lifecycle_meta))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tarballs/lifecycle-1.0.0.tgz"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(lifecycle_tarball, "application/octet-stream"),
        )
        .mount(&mock_server)
        .await;

    let temp_proj1 = tempdir().unwrap();
    let proj_path1 = temp_proj1.path();

    let mut cmd1 = AssertCmd::cargo_bin("bolt").unwrap();
    cmd1.current_dir(proj_path1)
        .arg("--registry")
        .arg(mock_server.uri())
        .arg("install")
        .arg("lifecycle-pkg");
    let assert_res = cmd1.assert().success();
    println!(
        "STDOUT:\n{}",
        String::from_utf8_lossy(&assert_res.get_output().stdout)
    );
    println!(
        "STDERR:\n{}",
        String::from_utf8_lossy(&assert_res.get_output().stderr)
    );
    assert!(
        proj_path1
            .join("node_modules")
            .join("lifecycle-pkg")
            .join("ran.txt")
            .exists()
    );

    let temp_proj2 = tempdir().unwrap();
    let proj_path2 = temp_proj2.path();

    let mut cmd2 = AssertCmd::cargo_bin("bolt").unwrap();
    cmd2.current_dir(proj_path2)
        .arg("--registry")
        .arg(mock_server.uri())
        .arg("install")
        .arg("--ignore-scripts")
        .arg("lifecycle-pkg");
    cmd2.assert().success();

    assert!(
        !proj_path2
            .join("node_modules")
            .join("lifecycle-pkg")
            .join("ran.txt")
            .exists()
    );
}

#[tokio::test]
async fn test_local_file_dependency_resolution() {
    let temp_proj = tempdir().unwrap();
    let proj_path = temp_proj.path();

    let local_lib_dir = proj_path.join("local-lib");
    fs::create_dir_all(&local_lib_dir).unwrap();
    let local_pkg_json = json!({
        "name": "my-local-lib",
        "version": "1.0.0",
        "main": "index.js"
    });
    fs::write(
        local_lib_dir.join("package.json"),
        local_pkg_json.to_string(),
    )
    .unwrap();
    fs::write(
        local_lib_dir.join("index.js"),
        "module.exports = 'HELLO_FROM_LOCAL_LIB';",
    )
    .unwrap();

    let app_dir = proj_path.join("app");
    fs::create_dir_all(&app_dir).unwrap();
    let app_pkg_json = json!({
        "name": "my-app",
        "version": "1.0.0",
        "dependencies": {
            "my-local-lib": "file:../local-lib"
        }
    });
    fs::write(app_dir.join("package.json"), app_pkg_json.to_string()).unwrap();

    let mut cmd = AssertCmd::cargo_bin("bolt").unwrap();
    cmd.current_dir(&app_dir).arg("install");
    cmd.assert().success();

    let test_script = r#"
        const local = require('my-local-lib');
        if (local !== 'HELLO_FROM_LOCAL_LIB') {
            process.exit(1);
        }
        console.log("LOCAL_FILE_DEP_SUCCESS");
    "#;

    let node_status = Command::new("node")
        .current_dir(&app_dir)
        .arg("-e")
        .arg(test_script)
        .status()
        .expect("Failed to execute node");

    assert!(node_status.success());
}

#[tokio::test]
async fn test_workspace_selection_flag() {
    let temp_proj = tempdir().unwrap();
    let proj_path = temp_proj.path();

    let pkg_a_dir = proj_path.join("packages").join("pkg-a");
    let pkg_b_dir = proj_path.join("packages").join("pkg-b");
    fs::create_dir_all(&pkg_a_dir).unwrap();
    fs::create_dir_all(&pkg_b_dir).unwrap();

    let root_pkg = json!({
        "name": "root-monorepo",
        "version": "1.0.0",
        "workspaces": ["packages/*"]
    });
    fs::write(proj_path.join("package.json"), root_pkg.to_string()).unwrap();

    let pkg_a_json = json!({
        "name": "pkg-a",
        "version": "1.0.0",
        "scripts": {
            "hello": "node -e \"console.log('PKG_A_HELLO')\""
        }
    });
    fs::write(pkg_a_dir.join("package.json"), pkg_a_json.to_string()).unwrap();

    let pkg_b_json = json!({
        "name": "pkg-b",
        "version": "1.0.0",
        "scripts": {
            "hello": "node -e \"console.log('PKG_B_HELLO')\""
        }
    });
    fs::write(pkg_b_dir.join("package.json"), pkg_b_json.to_string()).unwrap();

    let mut cmd = AssertCmd::cargo_bin("bolt").unwrap();
    cmd.current_dir(proj_path)
        .arg("-w")
        .arg("pkg-a")
        .arg("run")
        .arg("hello");

    let output = cmd.assert().success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(stdout.contains("PKG_A_HELLO"));
}

#[tokio::test]
async fn test_selective_update() {
    let mock_server = MockServer::start().await;

    let v1_tarball = create_test_tarball(&[
        ("package.json", br#"{"name":"sel-pkg","version":"1.0.0"}"#),
        ("index.js", br#"module.exports = 'v1';"#),
    ]);
    let v1_integrity = compute_sha512_integrity(&v1_tarball);

    let v2_tarball = create_test_tarball(&[
        ("package.json", br#"{"name":"sel-pkg","version":"1.1.0"}"#),
        ("index.js", br#"module.exports = 'v2';"#),
    ]);
    let v2_integrity = compute_sha512_integrity(&v2_tarball);

    let meta = json!({
        "name": "sel-pkg",
        "dist-tags": { "latest": "1.1.0" },
        "versions": {
            "1.0.0": {
                "name": "sel-pkg",
                "version": "1.0.0",
                "dist": {
                    "tarball": format!("{}/tarballs/sel-pkg-1.0.0.tgz", mock_server.uri()),
                    "integrity": v1_integrity
                }
            },
            "1.1.0": {
                "name": "sel-pkg",
                "version": "1.1.0",
                "dist": {
                    "tarball": format!("{}/tarballs/sel-pkg-1.1.0.tgz", mock_server.uri()),
                    "integrity": v2_integrity
                }
            }
        }
    });

    Mock::given(method("GET"))
        .and(path("/sel-pkg"))
        .respond_with(ResponseTemplate::new(200).set_body_json(meta))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tarballs/sel-pkg-1.0.0.tgz"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(v1_tarball, "application/octet-stream"),
        )
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tarballs/sel-pkg-1.1.0.tgz"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(v2_tarball, "application/octet-stream"),
        )
        .mount(&mock_server)
        .await;

    let temp_proj = tempdir().unwrap();
    let proj_path = temp_proj.path();
    let temp_cache = tempdir().unwrap();
    let initial_pkg = json!({
        "name": "app",
        "version": "1.0.0",
        "dependencies": {
            "sel-pkg": "^1.0.0"
        }
    });
    fs::write(proj_path.join("package.json"), initial_pkg.to_string()).unwrap();

    let mut cmd1 = AssertCmd::cargo_bin("bolt").unwrap();
    cmd1.current_dir(proj_path)
        .env("BOLT_CACHE_DIR", temp_cache.path())
        .arg("--registry")
        .arg(mock_server.uri())
        .arg("install")
        .arg("sel-pkg@1.0.0");
    cmd1.assert().success();

    let lock1 = fs::read_to_string(proj_path.join("package-lock.json")).unwrap();
    assert!(lock1.contains("\"version\": \"1.0.0\""));

    let mut cmd2 = AssertCmd::cargo_bin("bolt").unwrap();
    cmd2.current_dir(proj_path)
        .env("BOLT_CACHE_DIR", temp_cache.path())
        .arg("--registry")
        .arg(mock_server.uri())
        .arg("update")
        .arg("sel-pkg");
    let assert_out = cmd2.assert().success();
    println!(
        "UPDATE STDOUT:\n{}",
        String::from_utf8_lossy(&assert_out.get_output().stdout)
    );
    println!(
        "UPDATE STDERR:\n{}",
        String::from_utf8_lossy(&assert_out.get_output().stderr)
    );
    let lock2 = fs::read_to_string(proj_path.join("package-lock.json")).unwrap();
    println!("LOCK2:\n{}", lock2);
    assert!(lock2.contains("\"version\": \"1.1.0\""));
}

#[tokio::test]
async fn test_npm_and_bolt_lockfile_interoperability() {
    let mock_server = MockServer::start().await;

    let leaf_tarball = create_test_tarball(&[
        ("package.json", br#"{"name":"leaf-pkg","version":"1.0.0"}"#),
        ("index.js", br#"module.exports = 'LEAF';"#),
    ]);
    let leaf_integrity = compute_sha512_integrity(&leaf_tarball);

    let leaf_meta = json!({
        "name": "leaf-pkg",
        "dist-tags": { "latest": "1.0.0" },
        "versions": {
            "1.0.0": {
                "name": "leaf-pkg",
                "version": "1.0.0",
                "dist": {
                    "tarball": format!("{}/tarballs/leaf-1.0.0.tgz", mock_server.uri()),
                    "integrity": leaf_integrity
                }
            }
        }
    });

    Mock::given(method("GET"))
        .and(path("/leaf-pkg"))
        .respond_with(ResponseTemplate::new(200).set_body_json(leaf_meta))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tarballs/leaf-1.0.0.tgz"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(leaf_tarball, "application/octet-stream"),
        )
        .mount(&mock_server)
        .await;

    let temp_proj = tempdir().unwrap();
    let proj_path = temp_proj.path();

    let initial_pkg = json!({
        "name": "interop-app",
        "version": "1.0.0",
        "dependencies": {
            "leaf-pkg": "^1.0.0"
        }
    });
    fs::write(proj_path.join("package.json"), initial_pkg.to_string()).unwrap();

    let temp_cache = tempdir().unwrap();
    let mut bolt_cmd = AssertCmd::cargo_bin("bolt").unwrap();
    bolt_cmd
        .current_dir(proj_path)
        .env("BOLT_CACHE_DIR", temp_cache.path())
        .arg("--registry")
        .arg(mock_server.uri())
        .arg("install");
    bolt_cmd.assert().success();

    let lock_content = fs::read_to_string(proj_path.join("package-lock.json")).unwrap();
    let lock_json: serde_json::Value = serde_json::from_str(&lock_content).unwrap();
    assert_eq!(lock_json["lockfileVersion"], 3);
    assert!(lock_json["packages"]["node_modules/leaf-pkg"].is_object());

    fs::remove_dir_all(proj_path.join("node_modules")).unwrap();
    assert!(!proj_path.join("node_modules").exists());

    #[cfg(windows)]
    let npm_bin = "npm.cmd";
    #[cfg(not(windows))]
    let npm_bin = "npm";

    let npm_ci_status = Command::new(npm_bin)
        .current_dir(proj_path)
        .arg("ci")
        .status()
        .expect("Failed to execute npm ci");

    assert!(npm_ci_status.success());
    assert!(
        proj_path
            .join("node_modules")
            .join("leaf-pkg")
            .join("index.js")
            .exists()
    );

    let test_script = r#"
        const leaf = require('leaf-pkg');
        if (leaf !== 'LEAF') {
            process.exit(1);
        }
        console.log("INTEROP_SUCCESS");
    "#;

    let node_status = Command::new("node")
        .current_dir(proj_path)
        .arg("-e")
        .arg(test_script)
        .status()
        .expect("Failed to execute node");

    assert!(node_status.success());
}

#[tokio::test]
async fn test_install_tarball_url_dependency() {
    let mock_server = MockServer::start().await;

    let url_pkg_tarball = create_test_tarball(&[
        (
            "package.json",
            br#"{"name":"url-pkg","version":"1.2.3","main":"index.js"}"#,
        ),
        ("index.js", br#"module.exports = "URL_PKG_WORKED";"#),
    ]);

    Mock::given(method("GET"))
        .and(path("/tarballs/url-pkg-1.2.3.tgz"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(url_pkg_tarball)
                .insert_header("content-type", "application/octet-stream"),
        )
        .mount(&mock_server)
        .await;

    let temp_dir = tempdir().unwrap();
    let proj_path = temp_dir.path();
    let tarball_url = format!("{}/tarballs/url-pkg-1.2.3.tgz", mock_server.uri());

    let pkg_json = json!({
        "name": "tarball-url-test",
        "version": "1.0.0",
        "dependencies": {
            "url-pkg": tarball_url
        }
    });
    fs::write(
        proj_path.join("package.json"),
        serde_json::to_string_pretty(&pkg_json).unwrap(),
    )
    .unwrap();

    let mut cmd = AssertCmd::cargo_bin("bolt").unwrap();
    cmd.current_dir(proj_path).arg("install").assert().success();

    let installed_file = proj_path
        .join("node_modules")
        .join("url-pkg")
        .join("index.js");
    assert!(installed_file.exists());

    let node_status = Command::new("node")
        .current_dir(proj_path)
        .arg("-e")
        .arg("const val = require('url-pkg'); if (val !== 'URL_PKG_WORKED') process.exit(1);")
        .status()
        .expect("Failed to run node");
    assert!(node_status.success());
}

#[tokio::test]
async fn test_install_git_dependency() {
    // Create a local git repo
    let git_repo_dir = tempdir().unwrap();
    let repo_path = git_repo_dir.path();

    let pkg_json = json!({
        "name": "my-git-dep",
        "version": "2.0.0",
        "main": "index.js"
    });
    fs::write(
        repo_path.join("package.json"),
        serde_json::to_string_pretty(&pkg_json).unwrap(),
    )
    .unwrap();
    fs::write(
        repo_path.join("index.js"),
        "module.exports = 'GIT_SUCCESS';",
    )
    .unwrap();

    Command::new("git")
        .current_dir(repo_path)
        .arg("init")
        .status()
        .unwrap();
    Command::new("git")
        .current_dir(repo_path)
        .arg("config")
        .arg("user.name")
        .arg("Bolt Test")
        .status()
        .unwrap();
    Command::new("git")
        .current_dir(repo_path)
        .arg("config")
        .arg("user.email")
        .arg("test@bolt.dev")
        .status()
        .unwrap();
    Command::new("git")
        .current_dir(repo_path)
        .arg("add")
        .arg(".")
        .status()
        .unwrap();
    Command::new("git")
        .current_dir(repo_path)
        .arg("commit")
        .arg("-m")
        .arg("Initial commit")
        .status()
        .unwrap();

    let temp_dir = tempdir().unwrap();
    let proj_path = temp_dir.path();
    let git_url = format!("git+file://{}", repo_path.display()).replace('\\', "/");

    let app_pkg_json = json!({
        "name": "git-test-app",
        "version": "1.0.0",
        "dependencies": {
            "my-git-dep": git_url
        }
    });
    fs::write(
        proj_path.join("package.json"),
        serde_json::to_string_pretty(&app_pkg_json).unwrap(),
    )
    .unwrap();

    let mut cmd = AssertCmd::cargo_bin("bolt").unwrap();
    cmd.current_dir(proj_path).arg("install").assert().success();

    let installed_file = proj_path
        .join("node_modules")
        .join("my-git-dep")
        .join("index.js");
    assert!(installed_file.exists());

    let node_status = Command::new("node")
        .current_dir(proj_path)
        .arg("-e")
        .arg("const val = require('my-git-dep'); if (val !== 'GIT_SUCCESS') process.exit(1);")
        .status()
        .expect("Failed to run node");
    assert!(node_status.success());
}

#[tokio::test]
async fn test_workspace_recursive_run() {
    let temp_dir = tempdir().unwrap();
    let proj_path = temp_dir.path();

    // Create root package.json with workspaces
    let root_pkg_json = json!({
        "name": "monorepo-root",
        "version": "1.0.0",
        "workspaces": ["packages/*"]
    });
    fs::write(
        proj_path.join("package.json"),
        serde_json::to_string_pretty(&root_pkg_json).unwrap(),
    )
    .unwrap();

    // Create packages/pkg-a
    let pkg_a_dir = proj_path.join("packages").join("pkg-a");
    fs::create_dir_all(&pkg_a_dir).unwrap();
    let pkg_a_json = json!({
        "name": "pkg-a",
        "version": "1.0.0",
        "scripts": {
            "build": "node build.js"
        }
    });
    fs::write(
        pkg_a_dir.join("build.js"),
        "require('fs').writeFileSync('built_a.txt', 'OK');",
    )
    .unwrap();
    fs::write(
        pkg_a_dir.join("package.json"),
        serde_json::to_string_pretty(&pkg_a_json).unwrap(),
    )
    .unwrap();

    // Create packages/pkg-b
    let pkg_b_dir = proj_path.join("packages").join("pkg-b");
    fs::create_dir_all(&pkg_b_dir).unwrap();
    let pkg_b_json = json!({
        "name": "pkg-b",
        "version": "1.0.0",
        "scripts": {
            "build": "node build.js"
        }
    });
    fs::write(
        pkg_b_dir.join("build.js"),
        "require('fs').writeFileSync('built_b.txt', 'OK');",
    )
    .unwrap();
    fs::write(
        pkg_b_dir.join("package.json"),
        serde_json::to_string_pretty(&pkg_b_json).unwrap(),
    )
    .unwrap();

    let mut cmd = AssertCmd::cargo_bin("bolt").unwrap();
    cmd.current_dir(proj_path)
        .arg("--workspaces")
        .arg("run")
        .arg("build")
        .assert()
        .success();

    assert!(pkg_a_dir.join("built_a.txt").exists());
    assert!(pkg_b_dir.join("built_b.txt").exists());
}
#[tokio::test]
async fn test_npm_overrides_and_resolutions() {
    let mock_server = MockServer::start().await;

    let ov_v1_tarball = create_test_tarball(&[(
        "package.json",
        br#"{"name":"override-pkg","version":"1.0.0"}"#,
    )]);
    let ov_v1_integrity = compute_sha512_integrity(&ov_v1_tarball);

    let ov_v2_tarball = create_test_tarball(&[(
        "package.json",
        br#"{"name":"override-pkg","version":"2.0.0"}"#,
    )]);
    let ov_v2_integrity = compute_sha512_integrity(&ov_v2_tarball);

    let ov_meta = json!({
        "name": "override-pkg",
        "dist-tags": { "latest": "2.0.0" },
        "versions": {
            "1.0.0": {
                "name": "override-pkg",
                "version": "1.0.0",
                "dist": {
                    "tarball": format!("{}/override-pkg/-/override-pkg-1.0.0.tgz", mock_server.uri()),
                    "integrity": ov_v1_integrity
                }
            },
            "2.0.0": {
                "name": "override-pkg",
                "version": "2.0.0",
                "dist": {
                    "tarball": format!("{}/override-pkg/-/override-pkg-2.0.0.tgz", mock_server.uri()),
                    "integrity": ov_v2_integrity
                }
            }
        }
    });

    Mock::given(method("GET"))
        .and(path("/override-pkg"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ov_meta))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/override-pkg/-/override-pkg-1.0.0.tgz"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(ov_v1_tarball, "application/octet-stream"),
        )
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/override-pkg/-/override-pkg-2.0.0.tgz"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(ov_v2_tarball, "application/octet-stream"),
        )
        .mount(&mock_server)
        .await;

    let temp_proj = tempdir().unwrap();
    let proj_path = temp_proj.path();

    // Manifest specifies ^1.0.0, but overrides forces 2.0.0
    let pkg_json = json!({
        "name": "override-app",
        "version": "1.0.0",
        "dependencies": {
            "override-pkg": "^1.0.0"
        },
        "overrides": {
            "override-pkg": "2.0.0"
        }
    });
    fs::write(
        proj_path.join("package.json"),
        serde_json::to_string_pretty(&pkg_json).unwrap(),
    )
    .unwrap();

    let mut cmd = AssertCmd::cargo_bin("bolt").unwrap();
    cmd.current_dir(proj_path)
        .arg("--registry")
        .arg(mock_server.uri())
        .arg("install")
        .assert()
        .success();

    let installed_leaf_json: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(
            proj_path
                .join("node_modules")
                .join("override-pkg")
                .join("package.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(installed_leaf_json["version"], "2.0.0");
}

#[tokio::test]
async fn test_offline_mode() {
    let mock_server = MockServer::start().await;

    let leaf_tarball = create_test_tarball(&[(
        "package.json",
        br#"{"name":"cached-pkg","version":"1.0.0"}"#,
    )]);
    let leaf_integrity = compute_sha512_integrity(&leaf_tarball);

    let leaf_meta = json!({
        "name": "cached-pkg",
        "dist-tags": { "latest": "1.0.0" },
        "versions": {
            "1.0.0": {
                "name": "cached-pkg",
                "version": "1.0.0",
                "dist": {
                    "tarball": format!("{}/cached-pkg/-/cached-pkg-1.0.0.tgz", mock_server.uri()),
                    "integrity": leaf_integrity
                }
            }
        }
    });

    Mock::given(method("GET"))
        .and(path("/cached-pkg"))
        .respond_with(ResponseTemplate::new(200).set_body_json(leaf_meta))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/cached-pkg/-/cached-pkg-1.0.0.tgz"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(leaf_tarball, "application/octet-stream"),
        )
        .mount(&mock_server)
        .await;

    let temp_cache = tempdir().unwrap();
    let temp_proj = tempdir().unwrap();
    let proj_path = temp_proj.path();

    let pkg_json = json!({
        "name": "offline-app",
        "version": "1.0.0",
        "dependencies": {
            "cached-pkg": "^1.0.0"
        }
    });
    fs::write(
        proj_path.join("package.json"),
        serde_json::to_string_pretty(&pkg_json).unwrap(),
    )
    .unwrap();

    // 1. Initial warm install to populate cache
    let mut cmd1 = AssertCmd::cargo_bin("bolt").unwrap();
    cmd1.current_dir(proj_path)
        .env("BOLT_CACHE_DIR", temp_cache.path())
        .arg("--registry")
        .arg(mock_server.uri())
        .arg("install")
        .assert()
        .success();

    // Clean node_modules
    let _ = fs::remove_dir_all(proj_path.join("node_modules"));

    // 2. Install with --offline using a dead server URI
    let mut cmd2 = AssertCmd::cargo_bin("bolt").unwrap();
    cmd2.current_dir(proj_path)
        .env("BOLT_CACHE_DIR", temp_cache.path())
        .arg("--offline")
        .arg("--registry")
        .arg("http://127.0.0.1:9") // unreachable port
        .arg("install")
        .assert()
        .success();

    assert!(
        proj_path
            .join("node_modules")
            .join("cached-pkg")
            .join("package.json")
            .exists()
    );
}

#[tokio::test]
async fn test_catalog_protocol_resolution() {
    let mock_server = MockServer::start().await;

    let cat_tarball = create_test_tarball(&[(
        "package.json",
        br#"{"name":"catalog-pkg","version":"3.2.1"}"#,
    )]);
    let cat_integrity = compute_sha512_integrity(&cat_tarball);

    let cat_meta = json!({
        "name": "catalog-pkg",
        "dist-tags": { "latest": "3.2.1" },
        "versions": {
            "3.2.1": {
                "name": "catalog-pkg",
                "version": "3.2.1",
                "dist": {
                    "tarball": format!("{}/catalog-pkg/-/catalog-pkg-3.2.1.tgz", mock_server.uri()),
                    "integrity": cat_integrity
                }
            }
        }
    });

    Mock::given(method("GET"))
        .and(path("/catalog-pkg"))
        .respond_with(ResponseTemplate::new(200).set_body_json(cat_meta))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/catalog-pkg/-/catalog-pkg-3.2.1.tgz"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(cat_tarball, "application/octet-stream"),
        )
        .mount(&mock_server)
        .await;

    let temp_proj = tempdir().unwrap();
    let proj_path = temp_proj.path();

    let root_pkg_json = json!({
        "name": "catalog-workspace",
        "workspaces": ["packages/*"],
        "catalogs": {
            "default": {
                "catalog-pkg": "^3.2.0"
            }
        }
    });
    fs::write(
        proj_path.join("package.json"),
        serde_json::to_string_pretty(&root_pkg_json).unwrap(),
    )
    .unwrap();

    let pkg_a_dir = proj_path.join("packages").join("pkg-a");
    fs::create_dir_all(&pkg_a_dir).unwrap();
    let pkg_a_json = json!({
        "name": "pkg-a",
        "version": "1.0.0",
        "dependencies": {
            "catalog-pkg": "catalog:default"
        }
    });
    fs::write(
        pkg_a_dir.join("package.json"),
        serde_json::to_string_pretty(&pkg_a_json).unwrap(),
    )
    .unwrap();

    let mut cmd = AssertCmd::cargo_bin("bolt").unwrap();
    cmd.current_dir(proj_path)
        .arg("--registry")
        .arg(mock_server.uri())
        .arg("install")
        .assert()
        .success();

    let installed_json: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(
            proj_path
                .join("node_modules")
                .join("catalog-pkg")
                .join("package.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(installed_json["version"], "3.2.1");
}

#[tokio::test]
async fn test_bolt_audit_command() {
    let mock_server = MockServer::start().await;

    // Mock bulk security advisories endpoint
    let advisory_response = json!({
        "vulnerable-pkg": [
            {
                "id": 1092,
                "url": "https://github.com/advisories/GHSA-xxxx",
                "title": "Remote Code Execution in vulnerable-pkg",
                "severity": "high",
                "vulnerable_versions": "<2.0.0",
                "cwe": ["CWE-94"]
            }
        ]
    });

    Mock::given(method("POST"))
        .and(path("/-/npm/v1/security/advisories/bulk"))
        .respond_with(ResponseTemplate::new(200).set_body_json(advisory_response))
        .mount(&mock_server)
        .await;

    let temp_proj = tempdir().unwrap();
    let proj_path = temp_proj.path();

    let pkg_json = json!({
        "name": "audit-app",
        "version": "1.0.0"
    });
    fs::write(
        proj_path.join("package.json"),
        serde_json::to_string_pretty(&pkg_json).unwrap(),
    )
    .unwrap();

    let lock_json = json!({
        "name": "audit-app",
        "version": "1.0.0",
        "lockfileVersion": 3,
        "packages": {
            "": {
                "name": "audit-app",
                "version": "1.0.0"
            },
            "node_modules/vulnerable-pkg": {
                "version": "1.0.0"
            }
        }
    });
    fs::write(
        proj_path.join("package-lock.json"),
        serde_json::to_string_pretty(&lock_json).unwrap(),
    )
    .unwrap();

    // Audit with audit-level critical should exit 0 because severity is high
    let mut cmd1 = AssertCmd::cargo_bin("bolt").unwrap();
    cmd1.current_dir(proj_path)
        .arg("--registry")
        .arg(mock_server.uri())
        .arg("audit")
        .arg("--audit-level")
        .arg("critical")
        .assert()
        .success();

    // Audit with audit-level high should exit 1 because severity matches high
    let mut cmd2 = AssertCmd::cargo_bin("bolt").unwrap();
    cmd2.current_dir(proj_path)
        .arg("--registry")
        .arg(mock_server.uri())
        .arg("audit")
        .arg("--audit-level")
        .arg("high")
        .assert()
        .failure();
}

#[tokio::test]
async fn test_bolt_sbom_generation() {
    let temp_proj = tempdir().unwrap();
    let proj_path = temp_proj.path();

    let pkg_json = json!({
        "name": "sbom-demo",
        "version": "1.2.3",
        "dependencies": {
            "sample-dep": "^1.0.0"
        }
    });
    fs::write(
        proj_path.join("package.json"),
        serde_json::to_string_pretty(&pkg_json).unwrap(),
    )
    .unwrap();

    let lock_json = json!({
        "name": "sbom-demo",
        "version": "1.2.3",
        "lockfileVersion": 3,
        "packages": {
            "": {
                "name": "sbom-demo",
                "version": "1.2.3",
                "dependencies": {
                    "sample-dep": "^1.0.0"
                }
            },
            "node_modules/sample-dep": {
                "version": "1.0.0",
                "resolved": "https://registry.npmjs.org/sample-dep/-/sample-dep-1.0.0.tgz"
            }
        }
    });
    fs::write(
        proj_path.join("package-lock.json"),
        serde_json::to_string_pretty(&lock_json).unwrap(),
    )
    .unwrap();

    // Generate CycloneDX
    let cdx_out = proj_path.join("bom.cdx.json");
    let mut cmd1 = AssertCmd::cargo_bin("bolt").unwrap();
    cmd1.current_dir(proj_path)
        .arg("sbom")
        .arg("--format")
        .arg("cyclonedx")
        .arg("-o")
        .arg(&cdx_out)
        .assert()
        .success();

    assert!(cdx_out.exists());
    let cdx_content: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&cdx_out).unwrap()).unwrap();
    assert_eq!(cdx_content["bomFormat"], "CycloneDX");
    assert_eq!(cdx_content["components"][0]["name"], "sample-dep");

    // Generate SPDX
    let spdx_out = proj_path.join("bom.spdx.json");
    let mut cmd2 = AssertCmd::cargo_bin("bolt").unwrap();
    cmd2.current_dir(proj_path)
        .arg("sbom")
        .arg("--format")
        .arg("spdx")
        .arg("-o")
        .arg(&spdx_out)
        .assert()
        .success();

    assert!(spdx_out.exists());
    let spdx_content: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&spdx_out).unwrap()).unwrap();
    assert_eq!(spdx_content["spdxVersion"], "SPDX-2.3");
    assert!(
        spdx_content["packages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "sample-dep")
    );
}

#[tokio::test]
async fn test_bolt_audit_json_output() {
    let mock_server = MockServer::start().await;

    let advisory_response = json!({
        "vuln-lib": [
            {
                "id": 9999,
                "title": "Remote Code Execution",
                "module_name": "vuln-lib",
                "severity": "moderate",
                "vulnerable_versions": "<2.0.0",
                "patched_versions": ">=2.0.0",
                "url": "https://npmjs.com/advisories/9999"
            }
        ]
    });
    Mock::given(method("POST"))
        .and(path("/-/npm/v1/security/advisories/bulk"))
        .respond_with(ResponseTemplate::new(200).set_body_json(advisory_response))
        .mount(&mock_server)
        .await;

    let dir = tempdir().unwrap();
    let proj_path = dir.path();

    let pkg_json = json!({
        "name": "audit-json-app",
        "version": "1.0.0"
    });
    fs::write(
        proj_path.join("package.json"),
        serde_json::to_string_pretty(&pkg_json).unwrap(),
    )
    .unwrap();

    let lock_json = json!({
        "name": "audit-json-app",
        "version": "1.0.0",
        "lockfileVersion": 3,
        "packages": {
            "": {
                "name": "audit-json-app",
                "version": "1.0.0"
            },
            "node_modules/vuln-lib": {
                "version": "1.0.0"
            }
        }
    });
    fs::write(
        proj_path.join("package-lock.json"),
        serde_json::to_string_pretty(&lock_json).unwrap(),
    )
    .unwrap();

    let mut cmd = AssertCmd::cargo_bin("bolt").unwrap();
    let assert = cmd
        .current_dir(proj_path)
        .arg("--registry")
        .arg(mock_server.uri())
        .arg("audit")
        .arg("--format")
        .arg("json")
        .arg("--audit-level")
        .arg("high")
        .assert()
        .success();

    let output = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(parsed["metadata"]["vulnerabilities"]["total"], 1);
    assert!(parsed["advisories"]["vuln-lib"].is_array());
}

#[tokio::test]
async fn test_bolt_why_command() {
    let dir = tempdir().unwrap();
    let proj_path = dir.path();

    let pkg_json = json!({
        "name": "why-test-app",
        "version": "1.0.0",
        "dependencies": {
            "express": "^4.18.0"
        }
    });
    fs::write(
        proj_path.join("package.json"),
        serde_json::to_string_pretty(&pkg_json).unwrap(),
    )
    .unwrap();

    let lock_json = json!({
        "name": "why-test-app",
        "version": "1.0.0",
        "lockfileVersion": 3,
        "packages": {
            "": {
                "name": "why-test-app",
                "version": "1.0.0",
                "dependencies": {
                    "express": "^4.18.0"
                }
            },
            "node_modules/express": {
                "version": "4.18.2",
                "dependencies": {
                    "qs": "6.11.0"
                }
            },
            "node_modules/qs": {
                "version": "6.11.0"
            }
        }
    });
    fs::write(
        proj_path.join("package-lock.json"),
        serde_json::to_string_pretty(&lock_json).unwrap(),
    )
    .unwrap();

    // Test why qs (transitive dependency)
    let mut cmd = AssertCmd::cargo_bin("bolt").unwrap();
    let assert = cmd
        .current_dir(proj_path)
        .arg("why")
        .arg("qs")
        .arg("--json")
        .assert()
        .success();

    let output = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(parsed["package"], "qs");
    assert_eq!(parsed["reasons"][0]["requiredBy"], "express@4.18.2");
    assert_eq!(parsed["reasons"][0]["requirement"], "6.11.0");

    // Test why express (direct dependency)
    let mut cmd2 = AssertCmd::cargo_bin("bolt").unwrap();
    let assert2 = cmd2
        .current_dir(proj_path)
        .arg("why")
        .arg("express")
        .arg("--json")
        .assert()
        .success();

    let output2 = String::from_utf8(assert2.get_output().stdout.clone()).unwrap();
    let parsed2: serde_json::Value = serde_json::from_str(&output2).unwrap();
    assert_eq!(parsed2["package"], "express");
    assert_eq!(parsed2["reasons"][0]["requirement"], "^4.18.0");
}

#[tokio::test]
async fn test_bolt_outdated_command() {
    let mock_server = MockServer::start().await;

    let lodash_meta = json!({
        "name": "lodash",
        "dist-tags": { "latest": "4.17.21" },
        "versions": {
            "4.17.15": {
                "name": "lodash",
                "version": "4.17.15"
            },
            "4.17.21": {
                "name": "lodash",
                "version": "4.17.21"
            },
            "5.0.0": {
                "name": "lodash",
                "version": "5.0.0"
            }
        }
    });

    Mock::given(method("GET"))
        .and(path("/lodash"))
        .respond_with(ResponseTemplate::new(200).set_body_json(lodash_meta))
        .mount(&mock_server)
        .await;

    let dir = tempdir().unwrap();
    let temp_cache = tempdir().unwrap();
    let proj_path = dir.path();
    let pkg_json = json!({
        "name": "outdated-test-app",
        "version": "1.0.0",
        "dependencies": {
            "lodash": "^4.17.0"
        }
    });
    fs::write(
        proj_path.join("package.json"),
        serde_json::to_string_pretty(&pkg_json).unwrap(),
    )
    .unwrap();

    let lock_json = json!({
        "name": "outdated-test-app",
        "version": "1.0.0",
        "lockfileVersion": 3,
        "packages": {
            "": {
                "name": "outdated-test-app",
                "version": "1.0.0",
                "dependencies": {
                    "lodash": "^4.17.0"
                }
            },
            "node_modules/lodash": {
                "version": "4.17.15"
            }
        }
    });
    fs::write(
        proj_path.join("package-lock.json"),
        serde_json::to_string_pretty(&lock_json).unwrap(),
    )
    .unwrap();

    let mut cmd = AssertCmd::cargo_bin("bolt").unwrap();
    let assert = cmd
        .current_dir(proj_path)
        .env("BOLT_CACHE_DIR", temp_cache.path())
        .arg("--registry")
        .arg(mock_server.uri())
        .arg("outdated")
        .arg("--json")
        .assert()
        .success();

    let output = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(parsed["lodash"]["current"], "4.17.15");
    assert_eq!(parsed["lodash"]["wanted"], "4.17.21");
    assert_eq!(parsed["lodash"]["latest"], "4.17.21");
}
