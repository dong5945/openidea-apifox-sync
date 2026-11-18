//! Detects the project's code modules (Maven / Gradle) from build files.
//!
//! OpenIDEA has no project model to ask, so the build scripts are the source
//! of truth: every `pom.xml` is a module, and `settings.gradle(.kts)` lists
//! Gradle subprojects. This is deliberately text-level: no XML parser, no
//! Gradle evaluation.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeModule {
    /// Directory relative to the project root, forward slashes, `""` = root.
    pub path: String,
    /// Display name: the Maven artifactId, or the Gradle project name.
    pub name: String,
}

/// `build_files`: project-relative path and content of build scripts.
pub fn detect_modules(build_files: &[(String, String)]) -> Vec<CodeModule> {
    let mut out = Vec::new();
    for (path, content) in build_files {
        let lower = path.to_ascii_lowercase();
        if lower.ends_with("settings.gradle") || lower.ends_with("settings.gradle.kts") {
            out.extend(gradle_includes(content));
        } else if lower.ends_with("pom.xml") {
            let dir = parent_dir(path);
            let name = artifact_id(content).unwrap_or_else(|| {
                dir.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or("root").to_string()
            });
            out.push(CodeModule { path: dir, name });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out.dedup_by(|a, b| a.path == b.path);
    out
}

fn parent_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(i) => path[..i].to_string(),
        None => String::new(),
    }
}

/// The first `<artifactId>` in the pom (the project's own, before any
/// `<parent>`/`<dependencies>` in the usual layout).
fn artifact_id(content: &str) -> Option<String> {
    // `<parent>` comes first in most poms and its artifactId is not this module's.
    let own = strip_block(content, "<parent>", "</parent>");
    let start = own.find("<artifactId>")? + "<artifactId>".len();
    let end = own[start..].find("</artifactId>")? + start;
    let value = own[start..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn strip_block<'a>(text: &'a str, open: &str, close: &str) -> std::borrow::Cow<'a, str> {
    if let (Some(start), Some(close_at)) = (text.find(open), text.find(close)) {
        let end = close_at + close.len();
        if start <= close_at {
            let mut out = String::with_capacity(text.len());
            out.push_str(&text[..start]);
            out.push_str(&text[end..]);
            return std::borrow::Cow::Owned(out);
        }
    }
    std::borrow::Cow::Borrowed(text)
}

/// `include(":a", ":b:c")` / `include ':a', ':b:c'` -> modules `a`, `b/c`.
fn gradle_includes(content: &str) -> Vec<CodeModule> {
    let mut out = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if !t.starts_with("include") {
            continue;
        }
        let mut rest = &t["include".len()..];
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '(');
        for raw in rest.split(',') {
            let item = raw.trim().trim_end_matches(')').trim();
            let quoted = item.trim_matches(|c| c == '\'' || c == '"');
            if quoted.is_empty() {
                continue;
            }
            let path = quoted.trim_start_matches(':').replace(':', "/");
            if path.is_empty() {
                continue;
            }
            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
            out.push(CodeModule { path, name });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maven_poms_become_modules() {
        let files = vec![
            ("pom.xml".to_string(), "<project><artifactId>saas-erp</artifactId><modules><module>chergyi-sales</module></modules></project>".to_string()),
            ("chergyi-sales/pom.xml".to_string(), "<project><artifactId>chergyi-sales-api</artifactId></project>".to_string()),
        ];
        let mods = detect_modules(&files);
        assert_eq!(mods.len(), 2);
        assert_eq!(mods[0].path, "");
        assert_eq!(mods[0].name, "saas-erp");
        assert_eq!(mods[1].path, "chergyi-sales");
        assert_eq!(mods[1].name, "chergyi-sales-api");
    }

    #[test]
    fn parent_artifact_id_is_not_the_module_name() {
        let files = vec![(
            "child/pom.xml".to_string(),
            "<project><parent><groupId>g</groupId><artifactId>parent-name</artifactId></parent>\
             <artifactId>child-name</artifactId></project>"
                .to_string(),
        )];
        let mods = detect_modules(&files);
        assert_eq!(mods[0].name, "child-name");
    }
    #[test]
    fn gradle_settings_list_subprojects() {
        let files = vec![(
            "settings.gradle.kts".to_string(),
            "rootProject.name = \"erp\"\ninclude(\":chergyi-breed\", \":chergyi-finance\")\ninclude \":chergyi-sales-api\"\n".to_string(),
        )];
        let mods = detect_modules(&files);
        assert_eq!(
            mods.iter().map(|m| (m.path.as_str(), m.name.as_str())).collect::<Vec<_>>(),
            vec![
                ("chergyi-breed", "chergyi-breed"),
                ("chergyi-finance", "chergyi-finance"),
                ("chergyi-sales-api", "chergyi-sales-api"),
            ]
        );
    }

    #[test]
    fn npm_or_other_build_files_are_ignored() {
        let files = vec![("package.json".to_string(), "{\"name\":\"x\"}".to_string())];
        assert!(detect_modules(&files).is_empty());
    }
}