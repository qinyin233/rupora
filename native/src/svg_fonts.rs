use std::sync::{Arc, OnceLock};

use usvg::fontdb::{Database, Family, Query};

pub(crate) fn options() -> usvg::Options<'static> {
    static DATABASE: OnceLock<Arc<Database>> = OnceLock::new();
    let fontdb = DATABASE.get_or_init(|| {
        let mut database = Database::new();
        database.load_system_fonts();
        resolve_generic_families(&mut database);
        Arc::new(database)
    });
    usvg::Options {
        font_family: fontdb.family_name(&Family::Serif).to_owned(),
        fontdb: Arc::clone(fontdb),
        ..Default::default()
    }
}

fn resolve_generic_families(database: &mut Database) {
    // fontdb defaults (and fontconfig aliases) can name fonts absent from the
    // machine. usvg then drops SVG text, even when other usable fonts exist.
    let families: [(Family<'_>, &[&str]); 5] = [
        (
            Family::Serif,
            &[
                "Noto Serif",
                "DejaVu Serif",
                "Liberation Serif",
                "Times New Roman",
                "Times",
            ],
        ),
        (
            Family::SansSerif,
            &[
                "Noto Sans",
                "DejaVu Sans",
                "Liberation Sans",
                "Arial",
                "Helvetica",
            ],
        ),
        (
            Family::Monospace,
            &[
                "Noto Sans Mono",
                "DejaVu Sans Mono",
                "Liberation Mono",
                "Courier New",
                "Menlo",
            ],
        ),
        (Family::Cursive, &["Comic Sans MS", "Apple Chancery"]),
        (Family::Fantasy, &["Impact", "Papyrus"]),
    ];
    for (family, candidates) in families {
        if database
            .query(&Query {
                families: &[family],
                ..Default::default()
            })
            .is_some()
        {
            continue;
        }
        let names: Vec<_> = candidates.iter().map(|name| Family::Name(name)).collect();
        let fallback = database
            .query(&Query {
                families: &names,
                ..Default::default()
            })
            .and_then(|id| database.face(id))
            .or_else(|| {
                database
                    .faces()
                    .find(|face| face.monospaced == matches!(family, Family::Monospace))
            })
            .or_else(|| database.faces().next())
            .and_then(|face| face.families.first())
            .map(|(name, _)| name.clone());
        if let Some(name) = fallback {
            match family {
                Family::Serif => database.set_serif_family(name),
                Family::SansSerif => database.set_sans_serif_family(name),
                Family::Monospace => database.set_monospace_family(name),
                Family::Cursive => database.set_cursive_family(name),
                Family::Fantasy => database.set_fantasy_family(name),
                Family::Name(_) => unreachable!(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn portable_database() -> Database {
        let fonts = eframe::egui::FontDefinitions::default();
        let mut database = Database::new();
        for font in fonts.font_data.values() {
            database.load_font_data(font.font.to_vec());
        }
        database
    }

    #[test]
    fn generic_svg_text_renders_without_default_system_font_names() {
        let mut database = portable_database();
        assert!(
            database
                .query(&Query {
                    families: &[Family::SansSerif],
                    ..Default::default()
                })
                .is_none()
        );
        resolve_generic_families(&mut database);
        let options = usvg::Options {
            fontdb: Arc::new(database),
            ..Default::default()
        };
        for family in [
            "serif",
            "sans-serif",
            "monospace",
            "cursive",
            "fantasy",
            "missing-font",
        ] {
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="160" height="40"><text x="2" y="28" font-family="{family}" font-size="24">RUPORA</text></svg>"#
            );
            let tree = usvg::Tree::from_str(&svg, &options).unwrap();
            let mut pixmap = resvg::tiny_skia::Pixmap::new(160, 40).unwrap();
            resvg::render(
                &tree,
                resvg::tiny_skia::Transform::identity(),
                &mut pixmap.as_mut(),
            );
            assert!(
                pixmap.pixels().iter().any(|pixel| pixel.alpha() > 0),
                "{family} text disappeared"
            );
        }
    }

    #[test]
    fn existing_generic_font_choices_are_preserved() {
        let mut database = portable_database();
        let family = database.faces().next().unwrap().families[0].0.clone();
        database.set_sans_serif_family(&family);
        let before = database
            .query(&Query {
                families: &[Family::SansSerif],
                ..Default::default()
            })
            .unwrap();
        resolve_generic_families(&mut database);
        assert_eq!(database.family_name(&Family::SansSerif), family);
        assert_eq!(
            database.query(&Query {
                families: &[Family::SansSerif],
                ..Default::default()
            }),
            Some(before)
        );
    }
}
