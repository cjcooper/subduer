//! Manage Clusters: moving contexts within their kubeconfig file, what
//! removing one takes with it, and the dialog's wording.

use subduer_types::{ManagedContext, ManagedContexts, RemovedContext};

/// The names in order after moving `name` to index `to`, or `None` if that
/// leaves its file's run of rows (or changes nothing).
pub(crate) fn moved(list: &[ManagedContext], name: &str, to: usize) -> Option<Vec<String>> {
    let from = list.iter().position(|c| c.name == name)?;
    let (first, last) = file_bounds(list, from);
    if to == from || to < first || to > last {
        return None;
    }
    let mut names: Vec<String> = list.iter().map(|c| c.name.clone()).collect();
    let n = names.remove(from);
    names.insert(to, n);
    Some(names)
}

/// Puts the contexts in `order` (by name), as shown straight after a move.
pub(crate) fn reorder(contexts: &mut [ManagedContext], order: &[String]) {
    contexts.sort_by_key(|c| order.iter().position(|o| *o == c.name));
}

/// Where a row dropped on row `target` (in its lower half if `after`) ends up.
pub(crate) fn drop_index(from: usize, target: usize, after: bool) -> usize {
    let at = if after { target + 1 } else { target };
    if at > from { at - 1 } else { at }
}

/// Whether the pointer at `y` is over the lower half of a row from `top`
/// with height `height`: the moved row then goes after it.
pub(crate) fn drops_after(y: f64, top: f64, height: f64) -> bool {
    y > top + height / 2.0
}

/// What removing `name` takes with it: its cluster and user, unless another
/// listed context uses them too.
pub(crate) fn goes_with(list: &[ManagedContext], name: &str) -> (Option<String>, Option<String>) {
    let Some(c) = list.iter().find(|c| c.name == name) else { return (None, None) };
    let alone = |field: fn(&ManagedContext) -> &str| {
        let value = field(c);
        (!value.is_empty() && !list.iter().any(|o| o.name != name && field(o) == value)).then(|| value.to_owned())
    };
    (alone(|c| &c.cluster), alone(|c| &c.user))
}

/// The first and last rows of the file row `i` is in (the rows a context
/// can move among).
pub(crate) fn file_bounds(list: &[ManagedContext], i: usize) -> (usize, usize) {
    let file = &list[i].file;
    let first = list.iter().position(|c| c.file == *file).unwrap_or(i);
    let last = list.iter().rposition(|c| c.file == *file).unwrap_or(i);
    (first, last)
}

/// Whether the contexts come from more than one file: then each file's
/// rows get a heading.
pub(crate) fn several_files(l: &ManagedContexts) -> bool {
    l.files.iter().filter(|f| l.contexts.iter().any(|c| c.file == **f)).count() > 1
}

/// Whether the context being dragged is in `file`, so it can be dropped there.
pub(crate) fn drag_in_file(list: &[ManagedContext], dragging: Option<&str>, file: &str) -> bool {
    dragging.is_some_and(|d| list.iter().any(|x| x.name == d && x.file == file))
}

/// The API server shown under a context's name.
pub(crate) fn server_label(server: &str) -> String {
    if server.is_empty() { "No cluster entry".to_owned() } else { server.to_owned() }
}

/// The production toggle's tooltip.
pub(crate) fn prod_title(prod: bool) -> &'static str {
    if prod {
        "Production: changes need extra confirmation. Click to unmark."
    } else {
        "Not production. Click to mark it as production."
    }
}

pub(crate) fn prod_label(name: &str) -> String {
    format!("Production: {name}")
}

pub(crate) fn move_label(name: &str, up: bool) -> String {
    format!("Move {name} {}", if up { "up" } else { "down" })
}

/// Why Remove is disabled for a context in use, or nothing.
pub(crate) fn remove_title(here: bool, elsewhere: bool) -> &'static str {
    if here {
        "This window is on it: switch to another context first"
    } else if elsewhere {
        "Another window is on it: switch that window to another context first"
    } else {
        ""
    }
}

/// The question before removing `name`, saying what goes with it.
pub(crate) fn confirm_text(name: &str, cluster: Option<&str>, user: Option<&str>) -> String {
    let also = also(cluster, user);
    let also = match also.len() {
        0 => "Its cluster and user stay: other contexts use them.".to_owned(),
        1 => format!("Its {} goes too: nothing else uses it.", also[0]),
        _ => format!("Its {} go too: nothing else uses them.", also.join(" and ")),
    };
    format!("Remove {name:?} from the kubeconfig? {also} kubectl and other tools lose it too.")
}

pub(crate) fn remove_label(busy: bool) -> &'static str {
    if busy { "Removing…" } else { "Remove" }
}

/// What a removal took with it, said after it's done.
pub(crate) fn removed_text(r: &RemovedContext) -> String {
    let also = also(r.cluster.as_deref(), r.user.as_deref());
    if also.is_empty() { format!("Removed {:?}.", r.context) } else { format!("Removed {:?}, with its {}.", r.context, also.join(" and ")) }
}

/// `cluster "c"` and `user "u"`, for those that go.
fn also(cluster: Option<&str>, user: Option<&str>) -> Vec<String> {
    [cluster.map(|c| format!("cluster {c:?}")), user.map(|u| format!("user {u:?}"))].into_iter().flatten().collect()
}

#[cfg(test)]
#[path = "clusters_tests.rs"]
mod tests;
