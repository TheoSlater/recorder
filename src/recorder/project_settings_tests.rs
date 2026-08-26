use std::{fs, path::PathBuf};

use super::{
    AspectRatioPreset, CanvasBackground, CanvasBackgroundKind, CanvasComposition, CanvasView,
    ProjectSettings, load, save,
};
use crate::recorder::cursor_settings::{CursorStyle, MAX_CURSOR_SCALE};
use crate::recorder::project_settings::{MAX_CANVAS_ZOOM, MIN_CANVAS_ZOOM};
use crate::recorder::zoom::{
    CursorSizeEasing, CursorSizeRegion, ZoomEasing, ZoomRegion, ZoomTarget,
};

#[test]
fn saves_and_loads_cursor_settings() {
    let root =
        std::env::temp_dir().join(format!("recorder-project-settings-{}", std::process::id()));
    let path = root.join("project.json");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();

    let mut settings = ProjectSettings::default();
    settings.cursor.scale = 8.0;
    settings.cursor.style = CursorStyle::Circle;
    settings.canvas = CanvasView {
        zoom: 2.5,
        pan_x: 12.0,
        pan_y: -8.0,
    };
    let composition = CanvasComposition {
        aspect_ratio: AspectRatioPreset::Portrait,
        position_x: 0.15,
        position_y: -0.2,
        scale: 1.25,
        padding: 0.12,
        corner_radius: 0.08,
        shadow: true,
        background: CanvasBackground {
            kind: CanvasBackgroundKind::Gradient,
            solid_color: Some("#102030".to_string()),
            gradient_start: Some("#203040".to_string()),
            gradient_end: Some("#506070".to_string()),
            image_path: Some(PathBuf::from("background.png")),
        },
    };
    settings.canvas_composition = composition.clone();
    let region = ZoomRegion {
        start_us: 1_250_000,
        end_us: 3_750_000,
        scale: 2.25,
        target: ZoomTarget::CanvasCenter,
        easing: ZoomEasing::EaseInOut,
        zoom_in_end_us: Some(1_750_000),
        zoom_out_start_us: Some(3_250_000),
    };
    settings.zoom_regions.push(region);
    let cursor_region = CursorSizeRegion {
        start_us: 500_000,
        end_us: 1_500_000,
        start_scale: 1.0,
        end_scale: 2.0,
        easing: CursorSizeEasing::EaseInOut,
        ease_in_end_us: Some(700_000),
        ease_out_start_us: Some(1_300_000),
    };
    settings.cursor_size_regions.push(cursor_region);
    save(&path, &settings).unwrap();

    let loaded = load(&path);
    assert_eq!(loaded.schema_version, 5);
    assert_eq!(loaded.cursor.scale, MAX_CURSOR_SCALE);
    assert_eq!(loaded.cursor.style, CursorStyle::Circle);
    assert_eq!(
        loaded.canvas,
        CanvasView {
            zoom: 2.5,
            pan_x: 12.0,
            pan_y: -8.0
        }
    );
    assert_eq!(loaded.canvas_composition, composition);
    assert_eq!(loaded.zoom_regions, vec![region]);
    assert_eq!(loaded.cursor_size_regions, vec![cursor_region]);
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("\"target\": \"canvas_center\"")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn normalizes_canvas_view() {
    let view = CanvasView {
        zoom: 99.0,
        pan_x: f64::NAN,
        pan_y: f64::INFINITY,
    }
    .normalized();

    assert!((view.zoom - MAX_CANVAS_ZOOM).abs() < f64::EPSILON);
    assert_eq!(view.pan_x, 0.0);
    assert_eq!(view.pan_y, 0.0);

    let view = CanvasView {
        zoom: 0.0,
        pan_x: -40.0,
        pan_y: 80.0,
    }
    .normalized();
    assert!((view.zoom - MIN_CANVAS_ZOOM).abs() < f64::EPSILON);
    assert_eq!(view.pan_x, -40.0);
    assert_eq!(view.pan_y, 80.0);
}

#[test]
fn normalizes_canvas_composition() {
    let composition = CanvasComposition {
        aspect_ratio: AspectRatioPreset::Vertical,
        position_x: f64::INFINITY,
        position_y: -4.0,
        scale: 99.0,
        padding: -1.0,
        corner_radius: 2.0,
        shadow: true,
        background: CanvasBackground {
            kind: CanvasBackgroundKind::Gradient,
            solid_color: Some("not-a-colour".to_string()),
            gradient_start: Some("#abc".to_string()),
            gradient_end: Some("#AABBCCDD".to_string()),
            image_path: Some(PathBuf::new()),
        },
    }
    .normalized();

    assert_eq!(composition.aspect_ratio, AspectRatioPreset::Vertical);
    assert_eq!(composition.position_x, 0.0);
    assert_eq!(composition.position_y, -1.0);
    assert_eq!(composition.scale, super::MAX_COMPOSITION_SCALE);
    assert_eq!(composition.padding, 0.0);
    assert_eq!(composition.corner_radius, super::MAX_COMPOSITION_RADIUS);
    assert_eq!(composition.background.solid_color, None);
    assert_eq!(
        composition.background.gradient_start.as_deref(),
        Some("#abc")
    );
    assert_eq!(
        super::normalize_color(Some(" AABBCC ".to_string())).as_deref(),
        Some("#AABBCC")
    );
    assert_eq!(composition.background.image_path, None);
}

#[test]
fn saves_and_normalizes_motion_blur() {
    let root = std::env::temp_dir().join(format!(
        "recorder-motion-blur-settings-{}",
        std::process::id()
    ));
    let path = root.join("project.json");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();

    let mut settings = ProjectSettings::default();
    settings.motion_blur.amount = 0.6;
    save(&path, &settings).unwrap();
    assert_eq!(load(&path).motion_blur.amount, 0.6);

    settings.motion_blur.amount = 4.0;
    save(&path, &settings).unwrap();
    assert_eq!(load(&path).motion_blur.amount, 1.0);

    // A project written before motion blur existed keeps playing, and picks up
    // the subtle default rather than an unset value.
    fs::write(&path, br#"{"schema_version":4}"#).unwrap();
    assert_eq!(
        load(&path).motion_blur.amount,
        ProjectSettings::default().motion_blur.amount
    );

    let _ = fs::remove_dir_all(&root);
}
