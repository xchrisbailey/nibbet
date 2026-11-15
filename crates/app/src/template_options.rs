//! The Space's Templates as Insert Menu rows, after the block types.

use markdown_editor::{EditorCommand, InsertMenuOption};
use sett_store::Template;

/// The id the Insert Menu's Attachment option reports when picked.
pub(crate) const ATTACHMENT: &str = "attachment";

/// The block types, then the Attachment picker, then one row per Template (already sorted by title). A
/// Template's body never carries frontmatter, so nothing but the body is
/// inserted.
pub(crate) fn insert_options(templates: &[Template]) -> Vec<InsertMenuOption> {
    let mut options = InsertMenuOption::defaults();
    options.push(
        InsertMenuOption::new("Attachment", EditorCommand::Host(ATTACHMENT.into()))
            .aliases(["image", "file", "attach"]),
    );
    options.extend(templates.iter().map(|t| {
        InsertMenuOption::new(
            t.title.clone(),
            EditorCommand::InsertMarkdown(t.body.clone().into()),
        )
        .aliases(["template"])
        .section("Templates")
    }));
    options
}
