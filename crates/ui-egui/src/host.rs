//! The entry points a host that shows VectorCraft inside its own window needs (Bace, `apps/bace`).
//!
//! VectorCraft's own shells own a window and a file system: they open by path, write wherever a
//! picker says and keep preferences in their own storage. A host with none of that reaches the app
//! through this module: bytes in, bytes out, and the revision that tells an edit from a repaint.
//! Keeping it here leaves the shells' paths untouched, and the app's commands, journal and undo
//! history exactly as they are.

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::fileio::{self, SaveMode};

use crate::{Services, VectorcraftApp};

/// Where a hosted document is saved: a name the app can show and Save can write, which the host
/// never lets reach the file system — [`VectorcraftApp::save_svg_bytes`] keeps the bytes instead.
pub const HOST_PATH: &str = "bace/document.svg";

/// The app showing `bytes`, an SVG document called `name`, as the document in front.
///
/// The document opens before the app is made, because the app makes a view — the zoom, the fit,
/// the active artboard — for each document open when it is built. It has no path of its own: Save
/// in the app would ask a picker where to write, which a host that owns the bytes does not want.
/// It asks the app for them with [`VectorcraftApp::save_svg_bytes`] instead.
pub fn app(name: &str, bytes: &[u8]) -> Result<VectorcraftApp, String> {
    let mut session = Session::new();
    fileio::open_bytes(&mut session, name, bytes, None).map_err(|error| error.to_string())?;
    Ok(VectorcraftApp::new(session, Services::default()))
}

/// The app with no document open at all.
///
/// A host always shows something, so this is only for a host whose own starter document failed to
/// open: an app with nothing in it still draws its window, where a panic would take the host down.
pub fn empty() -> VectorcraftApp {
    VectorcraftApp::new(Session::new(), Services::default())
}

/// The drawing a window with no document of its own starts on: one artboard with a few shapes.
///
/// It is an SVG, so a first run opens it through the same path every other document takes, and it
/// gives the window something to show and to move before anything has ever been saved.
pub fn starter_svg() -> Vec<u8> {
    const STARTER: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600" viewBox="0 0 800 600">
  <title>Starter drawing</title>
  <rect x="64" y="88" width="280" height="200" rx="16" fill="#e8573f"/>
  <circle cx="520" cy="188" r="100" fill="#2f6fd0"/>
  <path d="M96 496 L288 356 L480 496 Z" fill="#f2b431"/>
  <rect x="520" y="432" width="200" height="24" rx="12" fill="#4a4a4a"/>
</svg>
"##;
    STARTER.as_bytes().to_vec()
}

impl VectorcraftApp {
    /// The document in front as `(id, revision)`, or `None` when no document is open.
    ///
    /// The revision moves on every change to the document's state — an edit, an undo, a save, and
    /// also a selection or a view change — so it says "redraw", not "modified". Which of those
    /// changes a save must carry is [`Self::document_is_dirty`]'s question.
    pub fn document_revision(&self) -> Option<(u64, u64)> {
        self.session.active().map(|document| (document.uid, document.revision))
    }

    /// Whether the document in front has changes its own Save would write.
    ///
    /// A document that was saved, or undone back to its last save, is clean even though its
    /// revision has moved since.
    pub fn document_is_dirty(&self) -> bool {
        self.session.active().is_some_and(|document| document.is_dirty())
    }

    /// The bytes Save would write for the document in front, as SVG.
    ///
    /// The save runs through the engine's own job — the one File ▸ Save runs — so the app's state
    /// afterwards is the state a save leaves it in: the document is no longer modified, and its
    /// path is [`HOST_PATH`]. Only the write itself is the host's: the bytes come back here
    /// instead of going to a file.
    pub fn save_svg_bytes(&mut self) -> Result<Vec<u8>, String> {
        let params = json!({ "path": HOST_PATH, "format": "svg" });
        let plan = fileio::save_plan(&self.session, SaveMode::Save, &params).map_err(|error| error.to_string())?;
        let mut saved = None;
        fileio::save_with(&mut self.session, plan, |path, bytes| {
            // A save can write beside the document (a file per artboard, a linked image): the
            // document itself is the SVG the plan names.
            if path.ends_with(".svg") {
                saved = Some(bytes.to_vec());
            }
            Ok(())
        })
        .map_err(|error| error.to_string())?;
        saved.ok_or_else(|| "the save wrote no SVG".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opened() -> VectorcraftApp {
        app("starter.svg", &starter_svg()).expect("the starter drawing opens")
    }

    fn shape_bounds(inspect: &serde_json::Value) -> Vec<[f64; 4]> {
        fn walk(node: &serde_json::Value, out: &mut Vec<[f64; 4]>) {
            let bounds = &node["bounds"];
            let number = |key: &str| bounds[key].as_f64().unwrap_or_default();
            out.push([number("x"), number("y"), number("width"), number("height")]);
            for child in node["children"].as_array().into_iter().flatten() {
                walk(child, out);
            }
        }
        let mut out = Vec::new();
        for layer in inspect["layers"].as_array().into_iter().flatten() {
            walk(layer, &mut out);
        }
        out
    }

    #[test]
    fn a_document_opens_and_saves_as_svg() {
        let mut vector = opened();
        assert!(vector.document_revision().is_some(), "a document is in front");
        let svg = vector.save_svg_bytes().expect("a save gives the SVG back");
        assert!(String::from_utf8_lossy(&svg).contains("<svg"), "the bytes a save wrote are an SVG");
        assert!(!vector.document_is_dirty(), "the save is the app's own save");
        assert!(app("saved.svg", &svg).is_ok(), "what it wrote opens again");
    }

    #[test]
    fn a_moved_shape_is_in_the_bytes_a_save_writes() {
        let mut vector = opened();
        vector.session.execute("shape.rectangle", &json!({ "x": 120.0, "y": 90.0, "width": 180.0, "height": 120.0 })).expect("a rectangle");
        vector.session.execute("object.move", &json!({ "dx": 40.0, "dy": 25.0 })).expect("a move");
        let svg = vector.save_svg_bytes().expect("a save");
        let mut reopened = app("saved.svg", &svg).expect("the saved bytes open");
        let inspect = reopened.session.execute("document.inspect", &json!({})).expect("an inspection");
        let bounds = shape_bounds(&inspect);
        assert!(
            bounds.iter().any(|shape| *shape == [160.0, 115.0, 180.0, 120.0]),
            "the moved rectangle is in the saved document, which holds {bounds:?}"
        );
    }
}
