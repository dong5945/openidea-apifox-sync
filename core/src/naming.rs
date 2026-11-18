//! Parameter/property naming conversion (`easyApiConfigurable.form.paramNameMethodConversion`).
//!
//! Names are split into words on case boundaries, `_` and `-`, then joined in
//! the requested style. Digits stay with the preceding word (`user2Name` ->
//! `user2`, `name`).

use crate::model::{ApiDoc, Field, Ty};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Naming {
    Original,
    Camel,
    Pascal,
    Snake,
    Kebab,
    Flat,
    Upper,
}

impl Naming {
    pub fn parse(value: &str) -> Self {
        match value {
            "camel" | "camelCase" | "小驼峰" => Self::Camel,
            "pascal" | "PascalCase" | "大驼峰" => Self::Pascal,
            "snake" | "snake_case" | "下划线" => Self::Snake,
            "kebab" | "kebab-case" => Self::Kebab,
            "flat" | "lowercase" | "全小写" => Self::Flat,
            "upper" | "UPPERCASE" | "全大写" => Self::Upper,
            _ => Self::Original,
        }
    }
}

pub fn convert(name: &str, naming: Naming) -> String {
    if naming == Naming::Original || name.is_empty() {
        return name.to_string();
    }
    let words = split_words(name);
    if words.is_empty() {
        return name.to_string();
    }
    let lower = |w: &str| w.to_ascii_lowercase();
    match naming {
        Naming::Original => name.to_string(),
        Naming::Camel => words
            .iter()
            .enumerate()
            .map(|(i, w)| {
                if i == 0 {
                    lower(w)
                } else {
                    capitalize(&lower(w))
                }
            })
            .collect(),
        Naming::Pascal => words.iter().map(|w| capitalize(&lower(w))).collect(),
        Naming::Snake => words.iter().map(|w| lower(w)).collect::<Vec<_>>().join("_"),
        Naming::Kebab => words.iter().map(|w| lower(w)).collect::<Vec<_>>().join("-"),
        Naming::Flat => words.iter().map(|w| lower(w)).collect(),
        Naming::Upper => words.iter().map(|w| w.to_ascii_uppercase()).collect::<Vec<_>>().join("_"),
    }
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// `userName`, `user_name`, `user-name`, `HTTPServer` -> words.
pub fn split_words(name: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = name.chars().collect();
    for (i, c) in chars.iter().copied().enumerate() {
        if c == '_' || c == '-' || c == ' ' {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }
        if c.is_ascii_uppercase() && !current.is_empty() {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).map(|n| n.is_ascii_lowercase()).unwrap_or(false);
            // `userName` -> split, `HTTPServer` -> split before `Server`.
            if prev.is_ascii_lowercase() || prev.is_ascii_digit() || (prev.is_ascii_uppercase() && next_lower) {
                words.push(std::mem::take(&mut current));
            }
        }
        current.push(c);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// Renames every property and parameter in place.
pub fn apply(doc: &mut ApiDoc, naming: Naming) {
    if naming == Naming::Original {
        return;
    }
    for ep in &mut doc.endpoints {
        for p in &mut ep.params {
            p.name = convert(&p.name, naming);
        }
        if let Some((_, ty)) = &mut ep.body {
            rename_ty(ty, naming);
        }
        if let Some(ty) = &mut ep.response {
            rename_ty(ty, naming);
        }
    }
    for ty in doc.schemas.values_mut() {
        rename_ty(ty, naming);
    }
}

fn rename_ty(ty: &mut Ty, naming: Naming) {
    match ty {
        Ty::Object(fields) => {
            for f in fields.iter_mut() {
                rename_field(f, naming);
            }
        }
        Ty::Array(inner) => rename_ty(inner, naming),
        _ => {}
    }
}

fn rename_field(field: &mut Field, naming: Naming) {
    field.name = convert(&field.name, naming);
    rename_ty(&mut field.ty, naming);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_camel_snake_and_acronyms() {
        assert_eq!(split_words("userName"), vec!["user", "Name"]);
        assert_eq!(split_words("user_name"), vec!["user", "name"]);
        assert_eq!(split_words("HTTPServer"), vec!["HTTP", "Server"]);
        assert_eq!(split_words("id"), vec!["id"]);
        assert_eq!(split_words("user2Name"), vec!["user2", "Name"]);
    }

    #[test]
    fn converts_between_styles() {
        assert_eq!(convert("user_name", Naming::Camel), "userName");
        assert_eq!(convert("user_name", Naming::Pascal), "UserName");
        assert_eq!(convert("userName", Naming::Snake), "user_name");
        assert_eq!(convert("UserName", Naming::Kebab), "user-name");
        assert_eq!(convert("userName", Naming::Flat), "username");
        assert_eq!(convert("userName", Naming::Upper), "USER_NAME");
        assert_eq!(convert("userName", Naming::Original), "userName");
    }
}