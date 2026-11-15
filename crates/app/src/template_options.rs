//! The Space's Templates as Insert Menu rows, after the block types.

use markdown_editor::{EditorCommand, InsertMenuOption};
use sett_store::Template;

/// What an Insert Menu row the host handles asks for. The id the editor
/// reports back is encoded and decoded only here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InsertAction {
    /// Pick files to attach.
    Attachment,
    /// Insert the Template with this slug.
    Template(String),
}

const ATTACHMENT_ID: &str = "attachment";
const TEMPLATE_PREFIX: &str = "template:";

impl InsertAction {
    fn id(&self) -> String {
        match self {
            InsertAction::Attachment => ATTACHMENT_ID.to_string(),
            InsertAction::Template(slug) => format!("{TEMPLATE_PREFIX}{slug}"),
        }
    }

    pub(crate) fn parse(id: &str) -> Option<InsertAction> {
        if id == ATTACHMENT_ID {
            return Some(InsertAction::Attachment);
        }
        id.strip_prefix(TEMPLATE_PREFIX)
            .map(|slug| InsertAction::Template(slug.to_string()))
    }
}

/// The block types, then the Attachment picker, then one row per Template
/// (already sorted by title). Picking either is the host's to handle, so a
/// Template's Attachments come along.
pub(crate) fn insert_options(templates: &[Template]) -> Vec<InsertMenuOption> {
    let mut options = InsertMenuOption::defaults();
    options.push(
        InsertMenuOption::new(
            "Attachment",
            EditorCommand::Host(InsertAction::Attachment.id().into()),
        )
        .aliases(["image", "file", "attach"]),
    );
    options.extend(templates.iter().map(|t| {
        InsertMenuOption::new(
            t.title.clone(),
            EditorCommand::Host(InsertAction::Template(t.slug.clone()).id().into()),
        )
        .aliases(["template"])
        .section("Templates")
    }));
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for action in [
            InsertAction::Attachment,
            InsertAction::Template("meeting-notes".into()),
        ] {
            assert_eq!(InsertAction::parse(&action.id()), Some(action));
        }
        assert_eq!(InsertAction::parse("other"), None);
    }
}
