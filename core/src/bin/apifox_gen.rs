//! `apifox-gen <dir>` — scan a Java project and print the OpenAPI document.
//! This is the same core the plugin runs; it exists so the export can be
//! inspected (and diffed in CI) without an IDE.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use apifox_core::{apifox, openapi, scan_modules};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(root) = args.next() else {
        eprintln!("用法: apifox-gen <项目目录> [--json 输出路径] [--project <Apifox项目ID>] [--branch <分支ID>] [--module <模块ID>]");
        return ExitCode::from(2);
    };
    let mut out: Option<PathBuf> = None;
    let mut project: Option<String> = None;
    let mut branch: Option<u64> = None;
    let mut module: Option<u64> = None;
    let mut code_module: Option<String> = None;
    let mut list_modules = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => out = args.next().map(PathBuf::from),
            "--project" => project = args.next(),
            "--branch" => branch = args.next().and_then(|v| v.parse().ok()),
            "--module" => module = args.next().and_then(|v| v.parse().ok()),
            "--code-module" => code_module = args.next(),
            "--modules" => list_modules = true,
            other => {
                eprintln!("未知参数: {other}");
                return ExitCode::from(2);
            }
        }
    }

    if list_modules {
        let root_path = Path::new(&root);
        let mut build: Vec<(String, String)> = Vec::new();
        for name in ["pom.xml", "settings.gradle", "settings.gradle.kts"] {
            for path in collect_named(root_path, name) {
                let rel = path
                    .strip_prefix(root_path)
                    .map(|p| {
                        p.components()
                            .map(|c| c.as_os_str().to_string_lossy().to_string())
                            .collect::<Vec<_>>()
                            .join("/")
                    })
                    .unwrap_or_default();
                if let Ok(text) = std::fs::read_to_string(&path) {
                    build.push((rel, text));
                }
            }
        }
        let modules = apifox_core::project::detect_modules(&build);
        println!("检测到 {} 个模块：", modules.len());
        for m in modules {
            println!("  {}  ->  {}", m.name, if m.path.is_empty() { "（根）" } else { m.path.as_str() });
        }
        return ExitCode::SUCCESS;
    }

    let files = match collect_java(Path::new(&root)) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("读取目录失败: {e}");
            return ExitCode::FAILURE;
        }
    };
    if files.is_empty() {
        eprintln!("{root} 下没有 .java 文件");
        return ExitCode::FAILURE;
    }
    let mut sources = Vec::with_capacity(files.len());
    for f in &files {
        match std::fs::read_to_string(f) {
            Ok(s) => sources.push(s),
            Err(e) => eprintln!("跳过 {}: {e}", f.display()),
        }
    }

    // `--code-module` scans one module but resolves types against all files.
    let doc = match &code_module {
        Some(module) => {
            let root_path = Path::new(&root);
            let pairs: Vec<(String, String)> = files
                .iter()
                .zip(sources.iter())
                .map(|(f, text)| {
                    let rel = f
                        .strip_prefix(root_path)
                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_else(|_| f.to_string_lossy().replace('\\', "/"));
                    (rel, text.clone())
                })
                .collect();
            scan_modules(&pairs, std::slice::from_ref(module))
                .into_iter()
                .next()
                .expect("one document per module")
        }
        None => {
            let root_path = Path::new(&root);
            let pairs: Vec<(String, String)> = files
                .iter()
                .zip(sources.iter())
                .map(|(f, text)| {
                    let rel = f
                        .strip_prefix(root_path)
                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_else(|_| f.to_string_lossy().replace('\\', "/"));
                    (rel, text.clone())
                })
                .collect();
            apifox_core::scan_project_files(&pairs)
        }
    };
    if doc.endpoints.is_empty() {
        eprintln!("扫描了 {} 个 Java 文件，未找到 Spring Controller", sources.len());
        return ExitCode::FAILURE;
    }
    println!("扫描 {} 个 Java 文件，得到 {} 个接口：", sources.len(), doc.endpoints.len());
    for ep in &doc.endpoints {
        println!("  {:<7} {:<40} {}", ep.method, ep.path, ep.summary);
    }
    println!(
        "数据模型 {} 个：{}",
        doc.schemas.len(),
        doc.schemas.keys().cloned().collect::<Vec<_>>().join(", ")
    );

    let json = openapi::render(&doc);
    match out {
        Some(path) => {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&path, &json) {
                eprintln!("写入 {} 失败: {e}", path.display());
                return ExitCode::FAILURE;
            }
            println!("已写入 {}", path.display());
        }
        None => println!("\n{json}"),
    }

    if let Some(id) = project {
        let opts = apifox::ImportOptions {
            branch_id: branch,
            module_id: module,
            ..apifox::ImportOptions::default()
        };
        let req = apifox::import_request(&id, &json, &opts);
        println!("\n将请求 POST {}", req.url);
        for (k, v) in &req.headers {
            println!("  {k}: {v}");
        }
        println!("  body: {} 字节", req.body.len());
        if let Some(b) = branch {
            println!("  目标分支 ID: {b}");
        }
        if let Some(m) = module {
            println!("  目标模块 ID: {m}");
        }
    }
    ExitCode::SUCCESS
}

fn collect_named(root: &Path, file_name: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if matches!(name, "target" | "build" | ".git" | "node_modules" | ".idea") {
                    continue;
                }
                stack.push(path);
            } else if path.file_name().and_then(|n| n.to_str()) == Some(file_name) {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}
fn collect_java(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if matches!(name, "target" | "build" | ".git" | "node_modules" | ".idea") {
                    continue;
                }
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("java") {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}