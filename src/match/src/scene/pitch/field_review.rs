//! Material previews and geometry checks for the runtime home surfaces.
use super::surface::{Pattern, Surface, catalogue};
use super::*;

#[test]
fn replay_style_ids_round_trip_and_old_documents_keep_the_original_field() {
    use crate::app::config::VenueInfo;
    for id in 1..=20 {
        let venue: VenueInfo =
            serde_json::from_value(serde_json::json!({"field_style": id})).unwrap();
        assert_eq!(venue.field_style, id);
        assert_eq!(
            Surface::from_id(id).colour(),
            Surface::from_id(venue.field_style).colour()
        );
    }
    let old: VenueInfo = serde_json::from_str("{}").unwrap();
    assert_eq!(old.field_style, 0);
    let original = Sward::mow(Upkeep::at(1.0), Pitch::TURF_TILE);
    for id in [old.field_style, 21, 255] {
        let fallback = Surface::from_id(id);
        assert_eq!(fallback.colour(), Vec3::ONE);
        assert_eq!(fallback.tile(), Pitch::TURF_TILE);
        let mesh = fallback.mesh(Upkeep::at(1.0));
        for attribute in [
            Mesh::ATTRIBUTE_POSITION,
            Mesh::ATTRIBUTE_COLOR,
            Mesh::ATTRIBUTE_UV_0,
        ] {
            assert_eq!(mesh.attribute(attribute), original.attribute(attribute));
        }
    }
}

#[test]
fn upkeep_changes_condition_without_changing_style_or_geometry() {
    use bevy::mesh::VertexAttributeValues;
    for id in 1..=20 {
        let surface = Surface::from_id(id);
        let kept = surface.mesh(Upkeep::at(1.0));
        for condition in [0.0, 0.5] {
            let worn = surface.mesh(Upkeep::at(condition));
            assert_eq!(
                kept.attribute(Mesh::ATTRIBUTE_POSITION),
                worn.attribute(Mesh::ATTRIBUTE_POSITION)
            );
            assert_eq!(
                kept.attribute(Mesh::ATTRIBUTE_UV_0),
                worn.attribute(Mesh::ATTRIBUTE_UV_0)
            );
            assert_ne!(
                kept.attribute(Mesh::ATTRIBUTE_COLOR),
                worn.attribute(Mesh::ATTRIBUTE_COLOR)
            );
            let Some(VertexAttributeValues::Float32x4(colours)) =
                worn.attribute(Mesh::ATTRIBUTE_COLOR)
            else {
                panic!()
            };
            assert!(colours.iter().flatten().all(|c| c.is_finite() && *c > 0.0));
        }
    }
}

#[test]
fn candidate_surfaces_are_complete_and_finite() {
    use bevy::mesh::VertexAttributeValues;
    let catalogue = catalogue();
    assert_eq!(catalogue.len(), 20);
    assert_eq!(
        catalogue
            .iter()
            .filter(|c| c.pattern == Pattern::Plain)
            .count(),
        10
    );
    let mut ids = std::collections::HashSet::new();
    for (index, candidate) in catalogue.iter().enumerate() {
        assert_eq!(
            candidate.id,
            (index + 1) as u8,
            "catalogue IDs must never be reordered"
        );
        assert!(ids.insert(candidate.id));
        assert!(matches!(
            candidate.pattern,
            Pattern::Plain | Pattern::Transverse | Pattern::Longitudinal
        ));
        assert_eq!(candidate.bands == 0, candidate.pattern == Pattern::Plain);
        assert_eq!(
            candidate.contrast == 0.0,
            candidate.pattern == Pattern::Plain
        );
        let mesh = Surface::from_id(index as u8 + 1).mesh(Upkeep::at(1.0));
        for attribute in [
            Mesh::ATTRIBUTE_POSITION,
            Mesh::ATTRIBUTE_NORMAL,
            Mesh::ATTRIBUTE_UV_0,
            Mesh::ATTRIBUTE_COLOR,
            Mesh::ATTRIBUTE_TANGENT,
        ] {
            let finite = match mesh.attribute(attribute).unwrap() {
                VertexAttributeValues::Float32x2(v) => v.iter().flatten().all(|x| x.is_finite()),
                VertexAttributeValues::Float32x3(v) => v.iter().flatten().all(|x| x.is_finite()),
                VertexAttributeValues::Float32x4(v) => v.iter().flatten().all(|x| x.is_finite()),
                _ => panic!("unexpected attribute"),
            };
            assert!(finite, "{} {}", candidate.id, candidate.name);
        }
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!()
        };
        let Some(Indices::U32(indices)) = mesh.indices() else {
            panic!()
        };
        let mut area = 0.0;
        for triangle in indices.chunks_exact(3) {
            let [a, b, c] = [
                positions[triangle[0] as usize],
                positions[triangle[1] as usize],
                positions[triangle[2] as usize],
            ]
            .map(Vec3::from);
            let normal = (b - a).cross(c - a);
            assert!(normal.y > 0.0, "{} has inverted triangles", candidate.id);
            area += normal.y as f64 * 0.5;
        }
        assert!((area - (Field::LENGTH * Field::WIDTH) as f64).abs() < 0.05);
        assert!(
            positions
                .iter()
                .all(|p| p[0].abs() <= Field::HALF_LENGTH + 0.0001
                    && p[2].abs() <= Field::HALF_WIDTH + 0.0001)
        );
    }
}

#[test]
#[ignore = "writes field review images; requires MATCH_FIELD_REVIEW"]
fn dump_field_candidates() {
    let directory = std::path::PathBuf::from(
        std::env::var("MATCH_FIELD_REVIEW").expect("set MATCH_FIELD_REVIEW"),
    );
    std::fs::create_dir_all(&directory).unwrap();
    let mut images = Assets::<Image>::default();
    let grass = Textures::turf(&mut images, Pitch::MOWN);
    let render = |id: &str, mesh: &Mesh| {
        let (width, height, pixels) = super::tests::overhead(
            mesh,
            &images,
            &grass,
            Vec2::new(-Field::HALF_LENGTH, -Field::HALF_WIDTH),
            Vec2::new(Field::HALF_LENGTH, Field::HALF_WIDTH),
            1260,
            5,
        );
        assert!(
            pixels.chunks_exact(4).all(|p| p[3] == 255),
            "{id}: uncovered pixels"
        );
        std::fs::write(directory.join(format!("{id}.rgba")), pixels).unwrap();
        println!("{id}: {width}x{height}");
    };
    render("00", &Sward::mow(Upkeep::at(1.0), Pitch::TURF_TILE));
    for candidate in catalogue() {
        render(
            &format!("{:02}", candidate.id),
            &candidate.mesh(Upkeep::at(1.0)),
        );
    }
}
