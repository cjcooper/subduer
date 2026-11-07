//! The About window's text: the third-party notices split into licenses,
//! their headings, the filter, and the sentences around them.

/// Whether a page's query string asks for the About window (`?about`).
pub(crate) fn is_about(search: &str) -> bool {
    search.trim_start_matches('?').split('&').any(|kv| kv == "about")
}

/// One license from `THIRD-PARTY-NOTICES.txt` and the software under it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Notice {
    pub(crate) title: String,
    /// `name version (repository)`, one per crate; empty for vendored code,
    /// whose title names it.
    pub(crate) used_by: Vec<String>,
    pub(crate) text: String,
}

const RULE: &str = "================================================================================";

/// Splits the generated notices into the intro (the counts) and the
/// licenses. See `about.hbs` and `tools/third-party-notices.sh`.
pub(crate) fn parse_notices(all: &str) -> (String, Vec<Notice>) {
    let separator = format!("{RULE}\n");
    let mut blocks = all.split(&separator);
    let intro = blocks.next().unwrap_or_default().trim().to_owned();
    let notices = blocks
        .filter_map(|block| {
            let (title, rest) = block.split_once('\n')?;
            let (used_by, text) = match rest.trim_start_matches('\n').strip_prefix("Used by:\n") {
                Some(rest) => {
                    let end = rest.find("\n\n").unwrap_or(rest.len());
                    let crates = rest[..end].lines().map(|l| l.trim().to_owned()).filter(|l| !l.is_empty()).collect();
                    (crates, &rest[end..])
                }
                None => (vec![], rest),
            };
            Some(Notice { title: title.trim().to_owned(), used_by, text: text.trim_matches('\n').to_owned() })
        })
        .collect();
    (intro, notices)
}

/// Whether `notice` matches the filter (case-insensitive, on its title or a
/// crate name).
pub(crate) fn matches(notice: &Notice, filter: &str) -> bool {
    let filter = filter.trim().to_lowercase();
    filter.is_empty()
        || notice.title.to_lowercase().contains(&filter)
        || notice.used_by.iter().any(|c| c.to_lowercase().contains(&filter))
}

/// The notices that match the filter, in order.
pub(crate) fn filtered(all: &[Notice], filter: &str) -> Vec<Notice> {
    all.iter().filter(|n| matches(n, filter)).cloned().collect()
}

/// A license's heading: its name and the first few crates under it (the same
/// license appears once per distinct copyright text).
pub(crate) fn summary(notice: &Notice) -> String {
    const SHOWN: usize = 3;
    let names: Vec<&str> = notice.used_by.iter().map(|c| c.split(' ').next().unwrap_or(c)).collect();
    match names.len() {
        0 => notice.title.clone(),
        n if n <= SHOWN => format!("{} · {}", notice.title, names.join(", ")),
        n => format!("{} · {} and {} more", notice.title, names[..SHOWN].join(", "), n - SHOWN),
    }
}

/// The per-license crate counts from the notices' intro.
pub(crate) fn counts(intro: &str) -> String {
    let lines: Vec<&str> = intro.lines().map(str::trim).filter(|l| l.starts_with(|c: char| c.is_ascii_digit())).collect();
    lines.join("\n")
}

/// The crates listed under all the licenses, and the vendored libraries
/// (which have a section of their own and no crate list).
pub(crate) fn totals(notices: &[Notice]) -> (usize, usize) {
    let crates = notices.iter().map(|n| n.used_by.len()).sum();
    let vendored = notices.iter().filter(|n| n.used_by.is_empty()).count();
    (crates, vendored)
}

pub(crate) fn version_text(version: &str) -> String {
    format!("Version {version}")
}

pub(crate) fn maker_text(maker: &str, copyright: &str) -> String {
    format!("An {maker} application. {copyright}")
}

pub(crate) fn license_intro(name: &str) -> String {
    format!("{name} is free software under the MIT License:")
}

pub(crate) fn third_party_text(name: &str, crates: usize, vendored: usize) -> String {
    format!(
        "{name} includes {crates} open-source Rust crates and {vendored} JavaScript libraries, used under the \
         licenses below. Each section lists the software it covers and the full license text."
    )
}

#[cfg(test)]
#[path = "about_tests.rs"]
mod tests;
