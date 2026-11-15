//! The Space's Templates as Insert Menu rows, after the block types.

use markdown_editor::{EditorCommand, InsertMenuOption};
use sett_store::Template;

/// The id the Insert Menu's Attachment option reports when picked.
pub(crate) const ATTACHMENT: &str = "attachment";

/// The id prefix a Template row reports when picked, followed by its slug.
pub(crate) const TEMPLATE_PREFIX: &str = "template:";

/// The block types, then the Attachment picker, then one row per Template (already sorted by title). Picking a
/// Template row is the host's to handle, so its Attachments come along.
pub(crate) fn insert_options(templates: &[Template]) -> Vec<InsertMenuOption> {
    let mut options = InsertMenuOption::defaults();
    options.push(
        InsertMenuOption::new("Attachment", EditorCommand::Host(ATTACHMENT.into()))
            .aliases(["image", "file", "attach"]),
    );
    options.extend(templates.iter().map(|t| {
        InsertMenuOption::new(
            t.title.clone(),
            EditorCommand::Host(format!("{TEMPLATE_PREFIX}{}", t.slug).into()),
        )
        .aliases(["template"])
        .section("Templates")
    }));
    options
}
