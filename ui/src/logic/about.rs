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

/// The notices that match the filter and, if one is picked, the license chip
/// (see [`kind`]), in order.
pub(crate) fn filtered(all: &[Notice], filter: &str, license: Option<&str>) -> Vec<Notice> {
    all.iter().filter(|n| matches(n, filter) && license.is_none_or(|l| kind(n) == l)).cloned().collect()
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

/// What a license chip filters on: the license's name, or `JavaScript` for
/// vendored code.
pub(crate) fn kind(notice: &Notice) -> &str {
    if notice.used_by.is_empty() && notice.title.starts_with("Vendored JavaScript") { "JavaScript" } else { &notice.title }
}

/// The license chips: each license (or `JavaScript`) and how much software it
/// covers, most first. A license appears once per distinct copyright text in
/// the notices, so the counts add those up.
pub(crate) fn chips(notices: &[Notice]) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = vec![];
    for n in notices {
        let count = n.used_by.len().max(1);
        match out.iter_mut().find(|(k, _)| k == kind(n)) {
            Some((_, c)) => *c += count,
            None => out.push((kind(n).to_owned(), count)),
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
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

/// The app's description as a sentence.
pub(crate) fn description_text(description: &str) -> String {
    format!("{}.", description.trim_end_matches('.'))
}

pub(crate) fn maker_text(maker: &str, copyright: &str) -> String {
    format!("An {maker} application. {copyright}. Released under the MIT License.")
}

/// What goes before the `i`th of `len` names in a sentence: nothing, a comma,
/// or " and " before the last.
pub(crate) fn list_separator(i: usize, len: usize) -> &'static str {
    match i {
        0 => "",
        i if i == len - 1 => " and ",
        _ => ", ",
    }
}

pub(crate) fn built_with_tail(crates: usize) -> String {
    format!(", and {crates} open-source Rust crates.")
}

pub(crate) fn third_party_tab(crates: usize, vendored: usize) -> String {
    format!("Third-party software ({})", crates + vendored)
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
