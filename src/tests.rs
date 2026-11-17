use crate::{
    model::{App, Input, InputMode, Preferences, demo_inventory},
    platform::{normalize_apps, normalize_inputs},
    state::{Direction, Effect, Overlay, Page, State, Target},
    storage, view,
};
use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf};

#[path = "recents-tests.rs"]
mod recent_tests;

fn ready() -> State {
    let mut state = State::new(Preferences::default());
    state.set_inventory(demo_inventory(), None);
    state
}

fn scene(state: &State) -> view::Scene {
    let assets = view::Assets {
        root: PathBuf::new(),
        version: "test".into(),
        metrics: BTreeMap::new(),
        artwork: crate::app_artwork::Service::new(false),
    };
    let navigation = view::navigation(&assets, state.preferences.language, state.play_available());
    view::scene_with_pill(
        state,
        &assets,
        navigation.items[state.page_index()].x,
        navigation.items[state.page_index()].w,
        time::OffsetDateTime::UNIX_EPOCH,
    )
}

fn controls(state: &State) -> Vec<crate::state::Control> {
    scene(state).controls
}

#[test]
fn favorites_edge_fragments_share_draw_hit_and_wheel_viewports() {
    let mut state = ready();
    let favorites: Vec<_> = state
        .inventory
        .apps
        .iter()
        .take(9)
        .map(|app| app.id.clone())
        .collect();
    state.preferences.favorites = Some(favorites.clone());
    assert_eq!(
        state.hit_test(&controls(&state), 1860., 600.),
        Some(Target::App(favorites[6].clone()))
    );
    assert!(state.wheel_scroll(&controls(&state), Some((1860., 600.)), 0, -1));
    assert!((state.home_scroll - crate::state::HOME_TILE_PITCH).abs() < 0.01);
    assert_eq!(state.scroll, 0.);
    assert_eq!(
        state.hit_test(&controls(&state), 40., 600.),
        Some(Target::App(favorites[0].clone()))
    );
    let scroll = state.home_scroll;
    state.pointer_focus(Target::App(favorites[0].clone()));
    assert_eq!(state.home_scroll, scroll);
    assert!(state.hit_test(&controls(&state), 4., 600.).is_none());
    assert!(state.hit_test(&controls(&state), 1915., 600.).is_none());
    assert!(state.hit_test(&controls(&state), 40., 1060.).is_none());
    let scene = scene(&state);
    let viewport = state.favorites_viewport();
    assert!(scene.draw.iter().any(|draw| matches!(draw, crate::renderer::Draw::Clip(Some(rect)) if rect.x == viewport.x && rect.w == viewport.w)));
    assert!(state.wheel_scroll(&scene.controls, Some((40., 600.)), 0, 1));
    assert_eq!(state.home_scroll, 0.);
    assert_eq!(state.scroll, 0.);
}

#[test]
fn widening_favorites_does_not_widen_apps_or_unclip_offscreen_rows() {
    let mut state = ready();
    state.open(1);
    let control = crate::state::Control {
        target: Target::App("sample".into()),
        rect: crate::state::Rect::new(0., 500., 1920., 600.),
        scrollable: true,
    };
    let controls = [control];
    assert!(state.hit_test(&controls, 40., 600.).is_none());
    assert!(state.hit_test(&controls, 1860., 600.).is_none());
    assert!(state.hit_test(&controls, 100., 1000.).is_none());
    assert_eq!(
        state.hit_test(&controls, 100., 600.),
        Some(Target::App("sample".into()))
    );
    assert!(!state.wheel_scroll(&controls, Some((1860., 600.)), 0, -1));
    assert_eq!(state.home_scroll, 0.);
    assert_eq!(state.scroll, 90.);
}

#[test]
fn remote_routes_from_header_and_back_through_settings() {
    let mut state = ready();
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::App("netflix".into()));
    state.direction(Direction::Up, &controls(&state));
    assert_eq!(state.focus, Target::Nav(0));
    state.open(2);
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Sidebar(0));
    state.activate();
    assert_eq!(state.focus, Target::Setting(0));
    assert!(state.back().is_empty());
    assert_eq!(state.focus, Target::Sidebar(0));
    state.back();
    assert_eq!(state.focus, Target::Nav(2));
    state.back();
    assert_eq!(state.page, Page::Home);
    assert_eq!(
        state.back(),
        vec![Effect::Recent {
            generation: 1,
            command: crate::recents::Command::List
        }]
    );
    assert!(state.recents_open());
}

#[test]
fn hiding_restores_nearest_app_and_hidden_filter_can_restore_it() {
    let mut state = ready();
    state.open(1);
    state.focus = Target::App("netflix".into());
    state.options();
    state.focus = Target::Option(1);
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(state.preferences.hidden.contains(&"netflix".into()));
    assert_eq!(state.focus, Target::App("youtube.leanback.v4".into()));
    state.filter = 5;
    assert_eq!(state.apps().len(), 1);
    state.focus = Target::App("netflix".into());
    state.options();
    state.focus = Target::Option(1);
    state.activate();
    assert!(state.preferences.hidden.is_empty());
    assert_eq!(state.focus, Target::App("netflix".into()));
    assert_eq!(state.filter, 0);
}

#[test]
fn reorder_persists_order_and_keeps_selected_identity() {
    let mut state = ready();
    state.focus = Target::App("netflix".into());
    state.hold();
    assert_eq!(
        state.direction(Direction::Right, &controls(&state)),
        vec![Effect::Save]
    );
    assert_eq!(
        &state.preferences.favorites(&state.inventory.apps)[..2],
        &["youtube.leanback.v4", "netflix"]
    );
    assert_eq!(state.focus, Target::App("netflix".into()));
    assert!(state.back().is_empty());
    assert!(state.moving.is_none());
}

#[test]
fn keyboard_is_modal_and_back_preserves_then_clears_search() {
    let mut state = ready();
    state.open(1);
    state.focus = Target::Search;
    state.activate();
    assert!(
        controls(&state)
            .iter()
            .all(|c| matches!(c.target, Target::Key(_) | Target::CloseDialog))
    );
    state.text("netflix");
    state.delete();
    state.text("x");
    state.back();
    assert_eq!(state.query, "netflix");
    assert_eq!(state.focus, Target::Search);
    assert_eq!(state.apps().len(), 1);
    state.back();
    assert!(state.query.is_empty());
    assert_eq!(state.focus, Target::Nav(1));
    assert_eq!(state.page, Page::Apps);
}

#[test]
fn partial_refresh_preserves_valid_inputs_and_never_invents_tv_apps() {
    let mut state = ready();
    state.set_inventory(
        crate::model::Inventory::default(),
        Some("Inputs unavailable".into()),
    );
    assert!(state.inventory.apps.is_empty());
    assert_eq!(state.inventory.inputs.len(), 3);
    assert_eq!(state.first_content(), Target::Browse);
}

#[test]
fn inventory_filters_system_apps_duplicates_and_unsafe_identifiers() {
    let apps = normalize_apps(&json!([
        {"id":"netflix","title":" Netflix\n"}, {"id":"netflix","title":"Duplicate"},
        {"id":"bad;command","title":"Bad"}, {"id":"com.webos.app.hidden","title":"Internal"},
        {"id":"com.webos.app.livetv","visible":false,"title":"Live TV"},
        {"id":"com.luma.launcher.native","visible":true},
        {"id":"com.webos.app.hdmi1","visible":true},
        {"id":"hidden","visible":"false"}
    ]))
    .unwrap();
    assert_eq!(apps.len(), 2);
    assert_eq!(
        apps.iter().find(|a| a.id == "netflix").unwrap().title,
        "Netflix"
    );
    assert!(normalize_apps(&json!({"apps":[]})).is_err());
    let inputs = normalize_inputs(&json!([
        {"appId":"com.webos.app.hdmi1","id":"HDMI_1","port":1,"connected":true},
        {"appId":"com.webos.app.hdmi2","id":"HDMI_4"},
        {"appId":"com.webos.app.hdmi3"}, {"appId":"com.webos.app.hdmi3"},
        {"appId":"com.webos.app.hdmi4","label":"Console"}
    ]))
    .unwrap();
    assert_eq!(inputs.len(), 2);
    assert_eq!(inputs[1].connected, None);
}

#[test]
fn inventory_visibility_cases_match_the_launcher_filters() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../tests/fixtures/app-visibility.json")).unwrap();
    let inventory: Vec<_> = cases.iter().map(|case| case["app"].clone()).collect();
    let actual: std::collections::BTreeSet<_> = normalize_apps(&json!(inventory))
        .unwrap()
        .into_iter()
        .map(|app| app.id)
        .collect();
    let expected: std::collections::BTreeSet<_> = cases
        .iter()
        .filter(|case| case["include"] == true)
        .map(|case| case["app"]["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn asset_loading_and_unknown_app_cards_do_not_require_collected_artwork() {
    let root = std::env::temp_dir().join(format!("luma-assets-no-dumps-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("version.txt"), "test").unwrap();
    std::fs::write(root.join("font-metrics.json"), "{}").unwrap();
    for webos in [false, true] {
        let assets = view::Assets::load(root.clone(), webos).unwrap();
        let mut state = State::new(Preferences::default());
        state.set_inventory(
            crate::model::Inventory {
                apps: vec![App::new("com.example.regional", "Regional App")],
                inputs: vec![],
            },
            None,
        );
        state.open(1);
        let scene = view::scene_with_pill(
            &state,
            &assets,
            800.,
            100.,
            time::OffsetDateTime::UNIX_EPOCH,
        );
        assert!(scene.draw.iter().any(|draw| matches!(draw,
            crate::renderer::Draw::TintedImage(path, ..) if path.ends_with("ui-icons/generic.lvec"))));
        assert!(scene.draw.iter().any(|draw| matches!(draw,
            crate::renderer::Draw::Text(value, ..) if value == "Regional App")));
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn auto_input_visibility_requires_a_positive_connection_report() {
    let mut preferences = Preferences::default();
    let mut input = Input {
        id: "com.webos.app.hdmi1".into(),
        title: "Console".into(),
        port: "HDMI 1".into(),
        connected: None,
    };
    assert!(!preferences.input_visible(&input));
    input.connected = Some(true);
    assert!(preferences.input_visible(&input));
    preferences
        .input_modes
        .insert(input.id.clone(), InputMode::Off);
    assert!(!preferences.input_visible(&input));
    preferences
        .input_modes
        .insert(input.id.clone(), InputMode::On);
    input.connected = Some(false);
    assert!(preferences.input_visible(&input));
}

#[test]
fn preferences_round_trip_and_corrupt_data_is_not_silently_overwritten() {
    let directory = std::env::temp_dir().join(format!("luma-native-test-{}", std::process::id()));
    let path = directory.join("preferences.json");
    let mut preferences = Preferences {
        favorites: Some(vec![]),
        background: "forest".into(),
        ..Preferences::default()
    };
    preferences
        .input_modes
        .insert("com.webos.app.hdmi1".into(), InputMode::Off);
    storage::save(&path, &preferences).unwrap();
    let loaded = storage::load(&path).unwrap();
    assert_eq!(loaded.favorites, Some(vec![]));
    assert_eq!(loaded.background, "forest");
    assert_eq!(loaded.input_modes["com.webos.app.hdmi1"], InputMode::Off);
    std::fs::write(&path, "broken").unwrap();
    assert!(storage::load(&path).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn gallery_and_grid_navigation_scroll_to_keep_focus_visible() {
    let mut state = ready();
    state.open(2);
    state.section = 1;
    state.focus = Target::Background(0);
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Background(3));
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Background(6));
    assert_eq!(
        state.scroll, 0.,
        "the web-sized gallery fits all three rows"
    );
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(state.focus, Target::Sidebar(1));
    state.open(1);
    let last = state.apps().last().unwrap().id.clone();
    state.focus(Target::App(last), &controls(&state));
    assert!(
        state.scroll > 0.,
        "library focus includes its wrapped caption"
    );
}

#[test]
fn cancelling_options_preserves_identity_and_does_not_save() {
    let mut state = ready();
    state.focus = Target::App("netflix".into());
    state.options();
    assert!(matches!(state.overlay, Some(Overlay::Options { .. })));
    state.focus = Target::CloseDialog;
    assert!(state.activate().is_empty());
    assert_eq!(state.focus, Target::App("netflix".into()));
    assert!(state.overlay.is_none());
}

#[test]
fn pointer_focus_does_not_scroll_the_clicked_control() {
    let mut state = ready();
    state.open(1);
    state.scroll = 150.;
    state.pointer_focus(Target::App("netflix".into()));
    assert_eq!(state.scroll, 150.);
    state.focus(Target::Nav(1), &controls(&state));
    assert_eq!(state.scroll, 0.);
}

#[test]
fn gallery_right_crosses_rows_in_gallery_order() {
    let mut state = ready();
    state.open(2);
    state.section = 1;
    state.focus = Target::Background(2);
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.focus, Target::Background(3));
}

#[test]
fn reordering_skips_hidden_and_unavailable_favorite_slots() {
    let mut state = ready();
    state.preferences.favorites = Some(vec![
        "netflix".into(),
        "missing".into(),
        "youtube.leanback.v4".into(),
        "amazon".into(),
    ]);
    state.preferences.hidden.push("youtube.leanback.v4".into());
    state.focus = Target::App("netflix".into());
    state.hold();
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(
        state.preferences.favorites.unwrap(),
        vec!["amazon", "missing", "youtube.leanback.v4", "netflix"]
    );
    assert_eq!(state.focus, Target::App("netflix".into()));
}

#[test]
fn focused_navigation_tabs_switch_pages_immediately() {
    let mut state = ready();
    // Home, Explore, Apps, Settings: Explore keeps the last page index but sits second.
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.page, Page::Explore);
    assert_eq!(state.focus, Target::Nav(crate::state::EXPLORE_PAGE));
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.page, Page::Apps);
    assert_eq!(state.focus, Target::Nav(1));
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.page, Page::Settings);
    assert_eq!(state.focus, Target::Nav(2));
    state.direction(Direction::Left, &controls(&state));
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(state.page, Page::Explore);
}

#[test]
fn greeting_subpage_opens_the_keyboard_and_back_restores_its_parent() {
    let mut state = ready();
    state.open(2);
    state.focus = Target::Setting(6);
    assert!(
        state
            .direction(Direction::Right, &controls(&state))
            .is_empty()
    );
    assert_eq!(state.subpage, Some(crate::state::Subpage::Greeting));
    assert_eq!(state.focus, Target::GreetingName);
    assert!(!state.text_input_active());
    assert!(state.activate().is_empty());
    assert!(state.text_input_active());
    state.text("Ana");
    assert_eq!(state.editing_text(), "Ana");
    assert!(state.preferences.greeting_name.is_empty());
    assert_eq!(state.finish_text_input(), vec![Effect::Save]);
    assert_eq!(state.focus, Target::GreetingName);
    assert!(state.finish_text_input().is_empty());
    assert_eq!(
        crate::i18n::Language::En.greeting(9, &state.preferences.greeting_name),
        "Good morning, Ana."
    );
    assert_eq!(state.subpage, Some(crate::state::Subpage::Greeting));
    assert!(state.back().is_empty());
    assert!(state.subpage.is_none());
    assert_eq!(state.focus, Target::Setting(6));
}

#[test]
fn greeting_name_replacement_commits_on_every_keyboard_completion_route() {
    for route in 0..3 {
        let mut state = ready();
        state.preferences.greeting_name = "Ana".into();
        state.open(2);
        state.focus = Target::Setting(6);
        state.activate();
        state.activate();
        assert!(state.editing_text().is_empty());
        state.text("  BobX ");
        state.delete_surrounding_text(-3, 3);
        assert_eq!(state.editing_text(), "  Bo");
        state.text("b  ");
        assert_eq!(state.preferences.greeting_name, "Ana");
        let effects = match route {
            0 => {
                state.overlay = Some(Overlay::SystemKeyboard);
                state.finish_text_input()
            }
            1 => state.back(),
            _ => {
                state.focus = Target::Key(crate::state::KEYS.len() - 1);
                state.activate()
            }
        };
        assert_eq!(effects, vec![Effect::Save]);
        assert_eq!(state.preferences.greeting_name, "Bob");
        assert_eq!(state.focus, Target::GreetingName);
        assert!(state.finish_text_input().is_empty());
        state.activate();
        state.text("   ");
        assert!(state.back().is_empty());
        assert_eq!(state.preferences.greeting_name, "Bob");
    }
}

#[test]
fn remote_can_clear_a_saved_greeting_name_without_moving_the_controls() {
    let mut state = ready();
    state.preferences.greeting_name = "Ana".into();
    state.open(2);
    state.focus = Target::Setting(6);
    state.activate();
    let before: Vec<_> = controls(&state)
        .iter()
        .map(|c| (c.target.clone(), c.rect.y))
        .collect();
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::GreetingNameClear);
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(state.preferences.greeting_name.is_empty());
    assert_eq!(state.focus, Target::GreetingName);
    let after: Vec<_> = controls(&state)
        .iter()
        .map(|c| (c.target.clone(), c.rect.y))
        .collect();
    assert_eq!(before, after);
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(state.focus, Target::Setting(6));
    assert!(state.subpage.is_none());
}

#[test]
fn app_name_settings_use_independent_defaults_including_with_legacy_preferences() {
    let old: Preferences = serde_json::from_value(json!({"show_names": true})).unwrap();
    assert!(!old.show_home_names);
    assert!(old.show_app_names);
    let mut state = ready();
    assert!(!state.preferences.show_home_names);
    assert!(state.preferences.show_app_names);
    state.open(2);
    state.focus = Target::Setting(0);
    assert!(
        state
            .direction(Direction::Right, &controls(&state))
            .is_empty()
    );
    assert_eq!(state.subpage, Some(crate::state::Subpage::AppNames));
    assert_eq!(state.focus, Target::AppNamesHome);
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(state.preferences.show_home_names);
    assert!(state.preferences.show_app_names);
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::AppNamesApps);
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(state.preferences.show_home_names);
    assert!(!state.preferences.show_app_names);
    assert_eq!(state.subpage, Some(crate::state::Subpage::AppNames));
    let restored: Preferences =
        serde_json::from_value(serde_json::to_value(&state.preferences).unwrap()).unwrap();
    assert!(restored.show_home_names);
    assert!(!restored.show_app_names);
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(state.focus, Target::Setting(0));
    assert!(state.subpage.is_none());
}

#[test]
fn home_and_apps_render_only_their_own_caption_preference() {
    let mut state = ready();
    for (home, apps) in [(false, true), (true, false), (true, true), (false, false)] {
        state.preferences.show_home_names = home;
        state.preferences.show_app_names = apps;
        for (page, expected) in [(0, home), (1, apps)] {
            state.open(page);
            let scene = scene(&state);
            let caption = scene.draw.iter().any(
                |draw| matches!(draw, crate::renderer::Draw::Text(value, ..) if value == "Netflix"),
            );
            assert_eq!(caption, expected, "Home={home}, Apps={apps}, page={page}");
            let card = scene
                .controls
                .iter()
                .find(|c| c.target == Target::App("netflix".into()))
                .unwrap();
            assert_eq!(
                card.rect.h > 154.4,
                expected,
                "Hidden captions must not reserve space"
            );
        }
    }
}

#[test]
fn input_name_actions_only_highlight_the_focused_row_and_keep_the_saved_mode() {
    let mut state = ready();
    state.preferences.reduced_motion = true;
    state
        .preferences
        .set_input_name("com.webos.app.hdmi1", "Console");
    state.open(2);
    state.section = crate::state::INPUTS_SECTION;
    state.focus = Target::InputMode("com.webos.app.hdmi1".into());
    state.activate();
    for target in [Target::InputRename, Target::InputNameReset] {
        state.pointer_focus(target.clone());
        let scene = scene(&state);
        let highlighted: Vec<_> = scene
            .draw
            .iter()
            .filter_map(|draw| match draw {
                crate::renderer::Draw::RoundedGradient(rect, ..)
                    if rect.x == 640. && rect.w == 1120. =>
                {
                    Some(rect.y)
                }
                _ => None,
            })
            .collect();
        let focused = scene.controls.iter().find(|c| c.target == target).unwrap();
        assert_eq!(highlighted, vec![focused.rect.y]);
        assert!(scene.draw.iter().any(|draw| matches!(draw, crate::renderer::Draw::TintedImage(path, ..) if path.ends_with("check.lvec"))));
        assert_eq!(
            state
                .preferences
                .input_modes
                .get("com.webos.app.hdmi1")
                .copied()
                .unwrap_or_default(),
            InputMode::Auto
        );
        assert!(scene.draw.iter().any(
            |draw| matches!(draw, crate::renderer::Draw::Text(value, ..) if value == "Clear name")
        ));
    }
}

#[test]
fn clock_choices_only_highlight_focus_while_preserving_saved_checkmarks() {
    let mut state = ready();
    state.preferences.reduced_motion = true;
    state.preferences.show_seconds = true;
    state.open(2);
    state.focus = Target::Setting(1);
    state.activate();
    state.activate();
    for index in 0..2 {
        state.pointer_focus(Target::Choice(index));
        let scene = scene(&state);
        let highlighted: Vec<_> = scene
            .draw
            .iter()
            .filter_map(|draw| match draw {
                crate::renderer::Draw::RoundedGradient(rect, ..)
                    if rect.x == 640. && rect.w == 1120. =>
                {
                    Some(rect.y)
                }
                _ => None,
            })
            .collect();
        let focused = scene
            .controls
            .iter()
            .find(|c| c.target == Target::Choice(index))
            .unwrap();
        assert_eq!(highlighted, vec![focused.rect.y]);
        let checks = scene.draw.iter().filter(|draw| matches!(draw, crate::renderer::Draw::TintedImage(path, ..) if path.ends_with("check.lvec"))).count();
        assert_eq!(checks, 1);
        assert!(!state.preferences.hour_24);
        assert!(state.preferences.show_seconds);
    }
}

#[test]
fn clock_format_subpage_saves_and_restores_its_parent_row() {
    let mut state = ready();
    state.open(2);
    state.focus = Target::Setting(1);
    assert!(
        state
            .direction(Direction::Right, &controls(&state))
            .is_empty()
    );
    assert_eq!(state.subpage, Some(crate::state::Subpage::Clock));
    assert_eq!(state.focus, Target::ClockFormat);
    assert!(
        state
            .direction(Direction::Right, &controls(&state))
            .is_empty()
    );
    assert_eq!(state.subpage, Some(crate::state::Subpage::ClockFormat));
    assert_eq!(state.focus, Target::Choice(0));
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(state.preferences.hour_24);
    assert!(!state.preferences.show_seconds);
    assert_eq!(state.subpage, Some(crate::state::Subpage::Clock));
    assert_eq!(state.focus, Target::ClockFormat);
    state.direction(Direction::Left, &controls(&state));
    assert!(state.subpage.is_none());
    assert_eq!(state.focus, Target::Setting(1));
    state.activate();
    state.activate();
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(state.subpage, Some(crate::state::Subpage::Clock));
    assert_eq!(state.focus, Target::ClockFormat);
    state.back();
    assert_eq!(state.focus, Target::Setting(1));
    assert!(state.subpage.is_none());
}

#[test]
fn time_format_selects_seconds_without_changing_the_hour_format() {
    let mut state = ready();
    state.preferences.hour_24 = true;
    state.open(2);
    state.focus = Target::Setting(1);
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.subpage, Some(crate::state::Subpage::Clock));
    assert!(!state.preferences.show_seconds);
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::ClockTimeFormat);
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.subpage, Some(crate::state::Subpage::TimeFormat));
    assert_eq!(state.focus, Target::Choice(0));
    assert!(
        !controls(&state)
            .iter()
            .any(|c| c.target == Target::Choice(2))
    );
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Choice(1));
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(state.preferences.show_seconds);
    assert!(state.preferences.hour_24);
    assert_eq!(state.subpage, Some(crate::state::Subpage::Clock));
    assert_eq!(state.focus, Target::ClockTimeFormat);
    state.activate();
    assert_eq!(state.focus, Target::Choice(1));
    state.back();
    assert_eq!(state.subpage, Some(crate::state::Subpage::Clock));
    assert_eq!(state.focus, Target::ClockTimeFormat);
    state.activate();
    state.direction(Direction::Up, &controls(&state));
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(!state.preferences.show_seconds);
    assert!(state.preferences.hour_24);
    state.back();
    assert!(state.subpage.is_none());
    assert_eq!(state.focus, Target::Setting(1));
}

#[test]
fn system_category_owns_refresh_and_quit_and_restores_sidebar_focus() {
    let mut state = ready();
    state.open(2);
    assert!(
        !controls(&state)
            .iter()
            .any(|c| matches!(c.target, Target::Setting(3 | 5)))
    );
    assert!(
        controls(&state)
            .iter()
            .any(|c| c.target == Target::Sidebar(crate::state::SYSTEM_SECTION))
    );
    state.focus = Target::Sidebar(crate::state::SYSTEM_SECTION - 1);
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Sidebar(crate::state::SYSTEM_SECTION));
    assert_eq!(state.section, crate::state::SYSTEM_SECTION);
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.focus, Target::Setting(3));
    assert_eq!(state.activate(), vec![Effect::Reload]);
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Setting(7));
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Setting(5));
    assert_eq!(state.activate(), vec![Effect::Quit]);
    state.back();
    assert_eq!(state.focus, Target::Sidebar(crate::state::SYSTEM_SECTION));
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Sidebar(0));
    assert_eq!(state.section, 0);
}

#[test]
fn text_rendering_preferences_default_to_msaa_and_recover_invalid_values() {
    use crate::text_rendering::TextRendering;
    for value in [
        json!({}),
        json!({"text_rendering": "unknown"}),
        json!({"text_rendering": null}),
        json!({"text_rendering": 4}),
    ] {
        let mut value = value;
        value["greeting_name"] = json!("Ana");
        let preferences: Preferences = serde_json::from_value(value).unwrap();
        assert_eq!(preferences.text_rendering, TextRendering::Msaa);
        assert_eq!(preferences.greeting_name, "Ana");
    }
}

#[test]
fn text_rendering_choices_persist_and_back_restores_the_system_row() {
    use crate::{state::Subpage, text_rendering::TextRendering};
    let mut state = ready();
    state.open(2);
    state.section = crate::state::SYSTEM_SECTION;
    state.focus = Target::Setting(7);
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.subpage, Some(Subpage::TextRendering));
    assert_eq!(state.focus, Target::Choice(0));
    assert!(scene(&state).draw.iter().any(|draw| matches!(
        draw,
        crate::renderer::Draw::Text(label, ..) if label == "Restart Luma to apply changes."
    )));
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Choice(1));
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert_eq!(state.preferences.text_rendering, TextRendering::Analytic);
    assert_eq!(state.focus, Target::Setting(7));
    assert!(state.subpage.is_none());
    let saved = serde_json::to_value(&state.preferences).unwrap();
    assert_eq!(saved["text_rendering"], "analytic");
    let mut restored = State::new(serde_json::from_value(saved).unwrap());
    restored.open(2);
    restored.section = crate::state::SYSTEM_SECTION;
    restored.focus = Target::Setting(7);
    restored.activate();
    assert_eq!(restored.focus, Target::Choice(1));
    restored.direction(Direction::Down, &controls(&restored));
    assert_eq!(restored.focus, Target::Choice(0));
    restored.back();
    assert_eq!(restored.focus, Target::Setting(7));
    assert_eq!(restored.preferences.text_rendering, TextRendering::Analytic);
    restored.activate();
    restored.direction(Direction::Up, &controls(&restored));
    assert_eq!(restored.activate(), vec![Effect::Save]);
    assert_eq!(restored.preferences.text_rendering, TextRendering::Msaa);
}

#[test]
fn input_visibility_has_explicit_choices_and_returns_to_the_same_input() {
    let mut state = ready();
    state.open(2);
    state.section = crate::state::INPUTS_SECTION;
    let id = "com.webos.app.hdmi1".to_string();
    state.focus = Target::InputMode(id.clone());
    state.activate();
    assert_eq!(state.focus, Target::Choice(2));
    state.direction(Direction::Up, &controls(&state));
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert_eq!(state.preferences.input_modes[&id], InputMode::Off);
    assert_eq!(state.focus, Target::InputMode(id.clone()));
    assert!(!state.preferences.input_visible(&state.inventory.inputs[0]));
    state.activate();
    state.back();
    assert_eq!(state.focus, Target::InputMode(id));
    assert!(state.subpage.is_none());
}

#[test]
fn background_entry_restores_the_applied_background() {
    let mut state = ready();
    state.open(2);
    state.preferences.background = "forest".into();
    state.focus = Target::Sidebar(1);
    state.section = 1;
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.focus, Target::Background(6));
    state.back();
    state.activate();
    assert_eq!(state.focus, Target::Background(6));
}

#[test]
fn categories_and_search_have_clear_actions_and_conditional_hidden_control() {
    let mut state = ready();
    state.open(1);
    assert!(
        !controls(&state)
            .iter()
            .any(|c| c.target == Target::Filter(5))
    );
    state.focus = Target::Search;
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(state.focus, Target::Filter(4));
    state.preferences.hidden.push("netflix".into());
    state.focus = Target::Search;
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(state.focus, Target::Filter(5));
    state.activate();
    assert_eq!(state.apps().len(), 1);
    state.focus = Target::ClearFilter;
    state.activate();
    assert_eq!(state.filter, 0);
    assert_eq!(state.focus, Target::Filter(1));
    state.query = "YouTube".into();
    state.focus = Target::ClearSearch;
    state.activate();
    assert!(state.query.is_empty());
    assert_eq!(state.focus, Target::Search);
}

#[test]
fn a_notice_does_not_escape_modal_focus_and_closing_restores_the_app() {
    let mut state = ready();
    state.open(1);
    state.notify("Saved");
    state.focus = Target::App("netflix".into());
    state.options();
    assert!(
        controls(&state)
            .iter()
            .all(|c| matches!(c.target, Target::Option(_) | Target::CloseDialog))
    );
    state.focus = Target::Option(1);
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::Option(1));
    state.back();
    assert_eq!(state.focus, Target::App("netflix".into()));
}

#[test]
fn remote_toolbar_focus_normalizes_scroll_but_hover_does_not() {
    let mut state = ready();
    state.open(1);
    state.scroll = 100.;
    state.pointer_focus(Target::Filter(1));
    assert_eq!(state.scroll, 100.);
    state.focus(Target::Filter(2), &controls(&state));
    assert_eq!(state.scroll, 0.);
}

#[test]
fn long_captions_expand_rows_without_overlapping_the_next_row() {
    let mut state = ready();
    state.open(1);
    state.inventory.apps.iter_mut().find(|app| app.id == "netflix").unwrap().title = "A very long installed application title that needs more than two lines to remain fully readable".into();
    let targets = controls(&state);
    let title = targets
        .iter()
        .find(|c| c.target == Target::App("netflix".into()))
        .unwrap();
    let next_id = state.apps()[6].id.clone();
    let next = targets
        .iter()
        .find(|c| c.target == Target::App(next_id.clone()))
        .unwrap();
    assert!(title.rect.h > 240.);
    assert!(next.rect.y > title.rect.y + title.rect.h);
}

#[test]
fn reduced_motion_disables_panel_wallpaper_and_notification_animation() {
    let mut state = ready();
    state.preferences.reduced_motion = true;
    state.open(2);
    state.focus = Target::Setting(1);
    state.activate();
    assert!(!state.animating());
    state.back();
    state.focus = Target::Background(6);
    state.activate();
    state.notify("Saved");
    assert!(!state.animating());
}

#[test]
fn pinning_a_hidden_app_restores_it_and_leaves_hidden_when_empty() {
    let mut state = ready();
    state.open(1);
    state.filter = 5;
    state.preferences.hidden.push("org.jellyfin.webos".into());
    state.focus = Target::App("org.jellyfin.webos".into());
    state.options();
    state.activate();
    assert!(state.preferences.hidden.is_empty());
    assert_eq!(state.filter, 0);
    assert!(
        state
            .preferences
            .favorites(&state.inventory.apps)
            .contains(&"org.jellyfin.webos".into())
    );
    assert_eq!(state.focus, Target::App("org.jellyfin.webos".into()));
}

#[test]
fn weather_location_keyboard_back_and_sidebar_routes_restore_focus() {
    let mut state = ready();
    state.open(2);
    state.focus = Target::Sidebar(crate::state::WEATHER_SECTION);
    state.section = crate::state::WEATHER_SECTION;
    state.activate();
    assert_eq!(state.focus, Target::WeatherToggle);
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::WeatherLocation);
    state.activate();
    assert_eq!(state.focus, Target::WeatherQuery);
    state.activate();
    state.text("Vienna");
    state.back();
    assert_eq!(state.focus, Target::WeatherSearch);
    assert_eq!(state.weather.query, "Vienna");
    assert_eq!(state.subpage, Some(crate::state::Subpage::WeatherLocation));
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(state.focus, Target::WeatherLocation);
    assert!(state.subpage.is_none());
}

#[test]
fn weather_units_save_without_replacing_celsius_cache_and_hide_keeps_city() {
    let mut state = ready();
    state.open(2);
    state.section = crate::state::WEATHER_SECTION;
    let place = crate::weather::Location {
        name: "Vienna".into(),
        region: "Vienna".into(),
        country: "Austria".into(),
        latitude: 48.2085,
        longitude: 16.3721,
    };
    state.preferences.weather_location = Some(place.clone());
    state.weather.cache = Some(crate::weather::Cache {
        key: place.key(),
        updated_at: 1000,
        reading: crate::weather::Reading {
            temperature: 14.2,
            code: 2,
            is_day: true,
        },
    });
    state.focus = Target::WeatherTemperature;
    state.activate();
    assert_eq!(state.focus, Target::Choice(0));
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert_eq!(
        state.preferences.temperature_unit,
        crate::weather::Unit::Fahrenheit
    );
    assert_eq!(
        state.weather.cache.as_ref().unwrap().reading.temperature,
        14.2
    );
    assert_eq!(state.focus, Target::WeatherTemperature);
    state.focus = Target::WeatherToggle;
    state.activate();
    assert!(!state.preferences.show_weather);
    assert_eq!(state.preferences.weather_location, Some(place));
}

#[test]
fn removing_weather_location_clears_cache_and_cancels_pending_results() {
    let mut state = ready();
    state.open(2);
    state.section = crate::state::WEATHER_SECTION;
    state.focus = Target::WeatherLocation;
    state.activate();
    state.focus = Target::WeatherClear;
    let effects = state.activate();
    assert_eq!(
        effects,
        vec![
            Effect::Save,
            Effect::Weather(crate::weather::Command::ClearCache)
        ]
    );
    assert!(state.weather.cache.is_none());
    assert!(state.preferences.weather_location.is_none());
    assert_eq!(state.focus, Target::WeatherSearch);
}

#[test]
fn language_selection_restores_its_parent_and_is_saved_with_preferences() {
    let mut state = ready();
    state.open(2);
    state.focus = Target::Setting(4);
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.subpage, Some(crate::state::Subpage::Language));
    assert_eq!(state.focus, Target::Choice(0));
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert_eq!(state.preferences.language, crate::i18n::Language::De);
    assert_eq!(state.focus, Target::Setting(4));
    state.activate();
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(state.focus, Target::Setting(4));
}

#[test]
fn remote_can_select_and_restore_every_language() {
    for (index, language) in crate::i18n::Language::ALL.into_iter().enumerate() {
        let mut state = ready();
        state.open(2);
        state.focus = Target::Setting(4);
        state.activate();
        for _ in 0..index {
            state.direction(Direction::Down, &controls(&state));
        }
        assert_eq!(state.focus, Target::Choice(index));
        assert_eq!(state.activate(), vec![Effect::Save]);
        assert_eq!(state.preferences.language, language);
        assert_eq!(state.focus, Target::Setting(4));

        let saved = serde_json::to_value(&state.preferences).unwrap();
        let mut restored = State::new(serde_json::from_value(saved).unwrap());
        restored.open(2);
        restored.focus = Target::Setting(4);
        restored.activate();
        assert_eq!(restored.preferences.language, language);
        assert_eq!(restored.focus, Target::Choice(index));
        restored.direction(Direction::Down, &controls(&restored));
        assert_eq!(
            restored.focus,
            Target::Choice((index + 1) % crate::i18n::Language::ALL.len())
        );
    }
}

#[test]
fn unsupported_language_preferences_recover_without_losing_other_choices() {
    let preferences: Preferences = serde_json::from_value(
        json!({"language":"unknown","favorites":["netflix"],"show_home_names":true}),
    )
    .unwrap();
    assert_eq!(preferences.language, crate::i18n::Language::En);
    assert!(preferences.show_home_names);
    assert_eq!(preferences.favorites, Some(vec!["netflix".into()]));
}

#[test]
fn unrelated_app_ids_and_titles_keep_their_installed_artwork() {
    for (id, title) in [
        ("com.tvweb.dashboard", "Glasshouse"),
        ("com.example.ardent", "Notes"),
        ("com.example.3ard", "Notes"),
        ("com.example.ard3", "Notes"),
        ("com.example.plexus", "Notes"),
        ("com.example.media", "Standard dashboard"),
        ("com.example.media", "netflixer"),
        ("com.example.media", "preNetflix"),
        ("amazon.alexa.view", "Amazon Alexa"),
        ("com.example.spotify", "Spotify tools"),
        ("com.example.media", "Plex"),
        ("com.example.browser", "Web Browser"),
        ("com.example.livetv", "Live TV"),
    ] {
        let app = App::new(id, title);
        assert_eq!(
            (app.brand.as_str(), app.category.as_str()),
            ("generic", "Utilities"),
            "An unrelated application must not be rebranded: {id} / {title}"
        );
    }
}

#[test]
fn curated_branding_does_not_use_title_words_or_partial_id_matches() {
    for (id, title) in [
        ("com.example.ard_more", "Notes"),
        ("com.example.more-ard", "Notes"),
        ("com.example.ARD", "Notes"),
        ("com.example.media", "ARD"),
        ("com.example.media", "(ARD)"),
        ("com.example.media", "ARD미디어테크"),
        ("com.example.media", "아에르데ARD"),
        ("com.example.media", "아에르데ARD미디어테크"),
        ("com.example.dashboard", "ARD"),
        ("com.example.media", "standard ardent ARD"),
    ] {
        let app = App::new(id, title);
        assert_eq!(
            (app.brand.as_str(), app.category.as_str()),
            ("generic", "Utilities"),
            "Only an exact app ID may select curated artwork: {id} / {title}"
        );
    }
}

#[test]
fn known_app_ids_preserve_the_curated_brands_and_categories() {
    let apps = demo_inventory().apps;
    let expected = [
        ("com.webos.app.livetv", "live-tv", "Streaming"),
        ("netflix", "netflix", "Streaming"),
        ("youtube.leanback.v4", "youtube", "Streaming"),
        ("com.apple.appletv", "apple-tv", "Streaming"),
        ("com.disney.disneyplus-prod", "disney-plus", "Streaming"),
        ("amazon", "prime-video", "Streaming"),
        ("plex", "plex", "Streaming"),
        ("spotify-beehive", "spotify", "Music"),
        ("org.jellyfin.webos", "jellyfin", "Streaming"),
        ("org.xbmc.kodi", "kodi", "Streaming"),
        ("com.limelight.webos", "moonlight", "Gaming"),
        ("org.webosbrew.hbchannel", "homebrew", "Utilities"),
        ("com.webos.app.browser", "browser", "Utilities"),
        ("com.arte.tv", "arte", "Streaming"),
        ("de.zdf.app.zdfm3", "zdf", "Streaming"),
        ("ard.mediathek", "das-erste", "Streaming"),
    ];
    for (id, brand, category) in expected {
        let app = apps.iter().find(|app| app.id == id).unwrap();
        assert_eq!(
            (app.brand.as_str(), app.category.as_str()),
            (brand, category),
            "Known app {id} must retain its curated brand and category"
        );
    }
}

#[test]
fn hdmi_plug_in_reports_the_cable_state_when_connected_only_means_present() {
    let inputs = normalize_inputs(&json!([
        {"id":"HDMI_1","label":"HDMI 1","port":1,"connected":true,"appId":"com.webos.app.hdmi1","hdmiPlugIn":false},
        {"id":"HDMI_2","label":"AVR-X1800H","port":2,"connected":true,"appId":"com.webos.app.hdmi2","hdmiPlugIn":true},
        {"id":"HDMI_3","label":"HDMI 3","port":3,"connected":true,"appId":"com.webos.app.hdmi3","hdmiPlugIn":false},
        {"id":"HDMI_4","label":"HDMI 4","port":4,"connected":true,"appId":"com.webos.app.hdmi4","hdmiPlugIn":false}
    ]))
    .unwrap();
    let connected: Vec<_> = inputs
        .iter()
        .map(|input| (input.id.as_str(), input.connected))
        .collect();
    assert_eq!(
        connected,
        [
            ("com.webos.app.hdmi1", Some(false)),
            ("com.webos.app.hdmi2", Some(true)),
            ("com.webos.app.hdmi3", Some(false)),
            ("com.webos.app.hdmi4", Some(false)),
        ]
    );
    assert_eq!(inputs[1].title, "AVR-X1800H");
    let preferences = Preferences::default();
    let visible: Vec<_> = inputs
        .iter()
        .filter(|input| preferences.input_visible(input))
        .map(|input| input.id.as_str())
        .collect();
    assert_eq!(visible, ["com.webos.app.hdmi2"]);
}

#[test]
fn inputs_without_hdmi_plug_in_keep_the_connected_report() {
    let inputs = normalize_inputs(&json!([
        {"appId":"com.webos.app.hdmi1","id":"HDMI_1","port":1,"connected":true},
        {"appId":"com.webos.app.hdmi2","id":"HDMI_2","port":2,"connected":false},
        {"appId":"com.webos.app.hdmi3","connected":true,"hdmiPlugIn":"yes"},
        {"appId":"com.webos.app.hdmi4"}
    ]))
    .unwrap();
    let connected: Vec<_> = inputs.iter().map(|input| input.connected).collect();
    assert_eq!(connected, [Some(true), Some(false), Some(true), None]);
}

#[test]
fn input_names_are_sanitized_and_old_preferences_load_without_them() {
    let old: Preferences = serde_json::from_value(json!({"background": "forest"})).unwrap();
    assert!(old.input_names.is_empty());
    let mut preferences: Preferences = serde_json::from_value(json!({
        "input_names": {
            "com.webos.app.hdmi1": "  Play\nStation\t5  ",
            "com.webos.app.hdmi2": "   ",
            "bad;id": "Ignored",
            "com.webos.app.hdmi3": "A".repeat(60),
        }
    }))
    .unwrap();
    preferences.sanitize();
    assert_eq!(
        preferences.input_names,
        BTreeMap::from([
            (
                "com.webos.app.hdmi1".to_string(),
                "PlayStation5".to_string()
            ),
            (
                "com.webos.app.hdmi3".to_string(),
                "A".repeat(crate::model::INPUT_NAME_LIMIT)
            ),
        ])
    );
    assert!(preferences.set_input_name("com.webos.app.hdmi2", " DENON "));
    assert!(!preferences.set_input_name("com.webos.app.hdmi2", "DENON"));
    assert_eq!(preferences.input_names["com.webos.app.hdmi2"], "DENON");
    assert!(preferences.set_input_name("com.webos.app.hdmi2", ""));
    assert!(!preferences.input_names.contains_key("com.webos.app.hdmi2"));
    assert!(!preferences.set_input_name("bad;id", "Name"));
}

#[test]
fn custom_input_names_keep_the_label_port_and_status_as_detail() {
    let mut preferences = Preferences::default();
    let input = Input {
        id: "com.webos.app.hdmi2".into(),
        title: "AVR-X1800H".into(),
        port: "HDMI 2".into(),
        connected: Some(true),
    };
    let language = preferences.language;
    let status = |preferences: &Preferences| {
        view::input_status(&input, preferences.input_name(&input).is_some(), language)
    };
    assert_eq!(preferences.input_title(&input), "HDMI 2");
    assert_eq!(status(&preferences), "AVR-X1800H · Connected");
    preferences.set_input_name(&input.id, "DENON");
    assert_eq!(preferences.input_title(&input), "DENON");
    assert_eq!(status(&preferences), "AVR-X1800H · HDMI 2 · Connected");
    let plain = Input {
        title: "HDMI 2".into(),
        ..input.clone()
    };
    assert_eq!(
        view::input_status(&plain, true, language),
        "HDMI 2 · Connected"
    );
    assert_eq!(view::input_status(&plain, false, language), "Connected");
}

#[test]
fn remote_renames_and_resets_an_input_from_its_settings_page() {
    let mut state = ready();
    state.open(2);
    state.section = crate::state::INPUTS_SECTION;
    let id = "com.webos.app.hdmi2".to_string();
    state.focus = Target::InputMode(id.clone());
    state.activate();
    assert_eq!(state.focus, Target::Choice(2));
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::InputRename);
    assert!(
        !controls(&state)
            .iter()
            .any(|c| c.target == Target::InputNameReset)
    );
    state.activate();
    assert!(matches!(state.overlay, Some(Overlay::Keyboard)));
    state.text("DENONX");
    state.delete();
    assert_eq!(state.back(), vec![Effect::Save]);
    assert_eq!(state.focus, Target::InputRename);
    assert_eq!(state.preferences.input_names[&id], "DENON");
    assert_eq!(
        state.preferences.input_title(&state.inventory.inputs[1]),
        "DENON"
    );
    // Dismissing an empty keyboard keeps the saved name.
    state.activate();
    assert!(state.finish_text_input().is_empty());
    assert_eq!(state.preferences.input_names[&id], "DENON");
    state.direction(Direction::Down, &controls(&state));
    assert_eq!(state.focus, Target::InputNameReset);
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert!(state.preferences.input_names.is_empty());
    assert_eq!(state.focus, Target::InputRename);
    state.direction(Direction::Up, &controls(&state));
    assert_eq!(state.focus, Target::Choice(2));
    state.focus = Target::InputRename;
    state.direction(Direction::Left, &controls(&state));
    assert_eq!(state.focus, Target::InputMode(id));
    assert!(state.subpage.is_none());
}

#[test]
fn keyboard_done_saves_a_typed_input_name() {
    let mut state = ready();
    state.open(2);
    state.section = crate::state::INPUTS_SECTION;
    state.focus = Target::InputMode("com.webos.app.hdmi1".into());
    state.activate();
    state.focus = Target::InputRename;
    state.activate();
    state.text("PS5");
    state.focus = Target::Key(
        crate::state::KEYS
            .iter()
            .position(|k| *k == "Done")
            .unwrap(),
    );
    assert_eq!(state.activate(), vec![Effect::Save]);
    assert_eq!(state.preferences.input_names["com.webos.app.hdmi1"], "PS5");
    assert!(state.overlay.is_none());
}

#[test]
fn home_lifecycle_replies_map_for_both_registration_versions() {
    use crate::platform::Lifecycle;
    // Lifecycle v2 (registerApp, in place of LG's Home executable) names it in "event".
    assert_eq!(
        Lifecycle::from_reply(&json!({"event": "relaunch", "reason": "hotKey"})),
        Some(Lifecycle::Relaunch)
    );
    assert_eq!(
        Lifecycle::from_reply(&json!({"event": "close"})),
        Some(Lifecycle::Close)
    );
    // Lifecycle v1 (registerNativeApp for a staged Home manifest on webOS 10/11) in "message".
    assert_eq!(
        Lifecycle::from_reply(&json!({
            "parameters": {"launchType": "homeKey", "activateType": "home"},
            "returnValue": true,
            "message": "relaunch"
        })),
        Some(Lifecycle::Relaunch)
    );
    assert_eq!(
        Lifecycle::from_reply(&json!({"returnValue": true, "message": "close"})),
        Some(Lifecycle::Close)
    );
    // Acknowledgements and keep-alive need no action.
    for reply in [
        json!({"returnValue": true, "message": "registered"}),
        json!({"event": "registered"}),
        json!({"event": "pause", "reason": "keepAlive"}),
        json!({"returnValue": false, "errorText": "refused"}),
    ] {
        assert_eq!(Lifecycle::from_reply(&reply), None, "{reply}");
    }
}

#[test]
fn foreground_check_needs_focus_after_a_relaunch_from_the_background() {
    use crate::platform::ForegroundCheck;
    use std::time::{Duration, Instant};
    let start = Instant::now();
    let late = start + ForegroundCheck::TIMEOUT;

    // Relaunch while hidden, then only Shown (SDL emits it for SDL_ShowWindow itself): fails.
    let mut check = ForegroundCheck::default();
    check.relaunched(start, false);
    assert!(!check.failed(start + Duration::from_secs(1)));
    assert!(check.failed(late));
    // Reported once.
    assert!(!check.failed(late + Duration::from_secs(1)));

    // FocusGained or a remote key in time: no failure.
    let mut check = ForegroundCheck::default();
    check.relaunched(start, false);
    check.confirmed();
    assert!(!check.failed(late));

    // Already focused and visible (Home pressed on Home): nothing to check.
    let mut check = ForegroundCheck::default();
    check.relaunched(start, true);
    assert!(!check.failed(late));

    // A later relaunch from the background is checked again.
    check.relaunched(late, false);
    assert!(check.failed(late + ForegroundCheck::TIMEOUT));
}

#[test]
fn prewarm_waits_while_recents_replaces_the_page() {
    let mut state = ready();
    let mut jobs = vec![Page::Settings, Page::Apps];
    state.open_recents();
    state.recents.visibility = 1.;
    // As after the frame that showed it.
    state.dirty = false;
    // The Apps scene would be Recents only, so preparing it now would be wasted.
    let apps_controls = |state: &mut State| {
        let current = std::mem::replace(&mut state.page, Page::Apps);
        let found = controls(state)
            .iter()
            .any(|control| matches!(control.target, Target::App(_)));
        state.page = current;
        found
    };
    assert!(!apps_controls(&mut state));
    assert_eq!(crate::next_prewarm(&mut jobs, &state, true), None);
    assert_eq!(jobs, [Page::Settings, Page::Apps]);
    // Once Recents is gone the retained jobs run, in order.
    state.overlay = None;
    state.recents.visibility = 0.;
    state.dirty = false;
    assert!(apps_controls(&mut state));
    assert_eq!(crate::next_prewarm(&mut jobs, &state, false), None);
    assert_eq!(
        crate::next_prewarm(&mut jobs, &state, true),
        Some(Page::Apps)
    );
    assert_eq!(
        crate::next_prewarm(&mut jobs, &state, true),
        Some(Page::Settings)
    );
    assert_eq!(crate::next_prewarm(&mut jobs, &state, true), None);
}

#[test]
fn failed_first_inventory_keeps_loading_until_retries_are_used_up() {
    let mut state = State::new(Preferences::default());
    state.loading = true;
    for attempt in 1..=crate::state::INVENTORY_RETRIES {
        assert_eq!(
            state.inventory_failed("The TV service timed out".into()),
            Some(crate::state::INVENTORY_RETRY_DELAY)
        );
        assert_eq!(state.inventory_failures, attempt);
        assert!(state.loading && state.error.is_none());
    }
    assert_eq!(
        state.inventory_failed("The TV service timed out".into()),
        None
    );
    assert!(!state.loading);
    assert_eq!(state.error.as_deref(), Some("The TV service timed out"));
    assert_eq!(state.first_content(), Target::Retry);
    // Retry by hand starts a fresh round of automatic retries.
    assert_eq!(state.inventory_failures, 0);
}

#[test]
fn inventory_success_resets_failures_and_later_failures_show_at_once() {
    let mut state = State::new(Preferences::default());
    state.loading = true;
    assert!(state.inventory_failed("timeout".into()).is_some());
    state.set_inventory(demo_inventory(), None);
    assert_eq!(state.inventory_failures, 0);
    state.loading = true;
    // Apps are on screen already: a failed refresh is reported, not retried behind them.
    assert_eq!(state.inventory_failed("timeout".into()), None);
    assert!(!state.loading && state.error.is_some());
    assert!(!state.inventory.apps.is_empty());
}

fn console_config() -> crate::console::Config {
    crate::console::Config {
        input: "com.webos.app.hdmi2".into(),
        title: "Hydra".into(),
        host: "192.168.4.112".into(),
        port: 8765,
        mac: "A8:A1:59:3B:4C:21".into(),
        broadcast: "192.168.7.255".into(),
        token: "secret".into(),
    }
}

fn console_ready() -> State {
    let mut state = State::new(Preferences {
        console: Some(console_config()),
        ..Preferences::default()
    });
    state.set_inventory(demo_inventory(), None);
    state
}

#[test]
fn console_settings_are_validated() {
    assert!(console_config().valid());
    for broken in [
        crate::console::Config {
            mac: "A8:A1:59:3B:4C".into(),
            ..console_config()
        },
        crate::console::Config {
            host: "pc.local".into(),
            ..console_config()
        },
        crate::console::Config {
            input: "netflix".into(),
            ..console_config()
        },
        crate::console::Config {
            token: String::new(),
            ..console_config()
        },
    ] {
        assert!(!broken.valid());
        let mut preferences = Preferences {
            console: Some(broken),
            ..Preferences::default()
        };
        preferences.sanitize();
        assert!(preferences.console.is_none());
    }
}

#[test]
fn console_magic_packet_repeats_the_mac() {
    let mac = crate::console::parse_mac("a8-a1-59-3b-4c-21").unwrap();
    let packet = crate::console::magic_packet(mac);
    assert_eq!(&packet[..6], &[0xff; 6]);
    assert!(packet[6..].chunks(6).all(|chunk| chunk == mac));
    assert_eq!(packet.len(), 102);
}

#[test]
fn console_tile_starts_the_transition() {
    let mut state = console_ready();
    // Not a favourite unless it is added: it is in Apps.
    assert!(
        state
            .apps()
            .iter()
            .all(|app| app.id != crate::console::APP_ID)
    );
    // Added to the favourites, it works from Home.
    state.preferences.favorites = Some(vec![crate::console::APP_ID.into(), "netflix".into()]);
    assert_eq!(state.apps()[0].id, crate::console::APP_ID);
    state.focus = Target::App(crate::console::APP_ID.into());
    let effects = state.activate();
    assert!(matches!(effects[..], [Effect::ConsoleStart(1)]));
    assert!(state.console.is_some());
    // Arrows do nothing while it runs; Back cancels and restores the tile's focus.
    state.direction(Direction::Right, &[]);
    assert_eq!(state.focus, Target::App(crate::console::APP_ID.into()));
    state.back();
    assert!(state.console.is_none());
    assert_eq!(state.focus, Target::App(crate::console::APP_ID.into()));
}

#[test]
fn console_ready_waits_for_the_transition_then_switches_once() {
    use crate::console::{FADE, MIN_SHOW, Phase, READY_HOLD, Session};
    let start = std::time::Instant::now();
    let mut session = Session::new(1, Target::Nav(0), Target::Nav(0), None, start);
    session.update(Phase::Ready, start);
    assert_eq!(session.ready_at, Some(start + MIN_SHOW));
    // Ready holds (the burst and chime), then fades, then switches.
    assert!(!session.launch_due(start + MIN_SHOW + FADE));
    assert_eq!(session.fade(start + MIN_SHOW + READY_HOLD), 0.);
    let due = start + MIN_SHOW + READY_HOLD + FADE;
    assert!(session.launch_due(due));
    session.launched_at = Some(due);
    assert!(!session.launch_due(due + FADE));
}

#[test]
fn console_times_out_to_switch_anyway() {
    use crate::console::{Phase, READY_TIMEOUT, Session};
    let start = std::time::Instant::now();
    let mut session = Session::new(1, Target::Nav(0), Target::Nav(0), None, start);
    assert!(session.tick(start + READY_TIMEOUT));
    assert_eq!(session.phase, Phase::Failed);
    // A late answer no longer changes it; Switch anyway does.
    session.update(Phase::Ready, start + READY_TIMEOUT);
    assert_eq!(session.phase, Phase::Failed);
    session.force(start + READY_TIMEOUT);
    assert!(session.ready_at.is_some());
}

#[test]
fn console_tile_without_settings_is_absent() {
    let state = ready();
    assert!(
        !state
            .apps()
            .iter()
            .any(|app| app.id == crate::console::APP_ID)
    );
}

// ---- The Play page ----

fn play_ready() -> State {
    let mut state = console_ready();
    state.set_console_library(crate::console::demo_library());
    state
}

fn game_json(key: &str) -> serde_json::Value {
    json!({
        "key": key,
        "title": format!("Game {key}"),
        "playTimeMs": 3_600_000,
        "lastPlayed": "2026-11-01T10:00:00.000Z",
        "achievements": {"unlocked": 3, "total": 10},
        "art": ["cover", "hero", "shot0", "shot9", "poster"],
    })
}

#[test]
fn console_library_is_bounded_and_cleaned() {
    let mut games: Vec<_> = (0..20).map(|n| game_json(&format!("steam:{n}"))).collect();
    games.insert(0, game_json("bad key/../x"));
    games.insert(1, game_json("steam:1"));
    games[1]["title"] = json!("Line\nbreak\u{7}  and   spaces");
    games[1]["playTimeMs"] = json!(-5);
    games[1]["lastPlayed"] = json!("yesterday");
    games[1]["achievements"] = json!({"unlocked": 50, "total": 10});
    games[1]["description"] = json!("x".repeat(2000));
    let library = crate::console::Library::parse(
        serde_json::to_vec(&json!({"awake": true, "games": games}))
            .unwrap()
            .as_slice(),
    )
    .unwrap();
    assert_eq!(library.games.len(), crate::console::GAMES);
    assert!(
        library
            .games
            .iter()
            .all(|game| crate::console::valid_key(&game.key))
    );
    let keys: std::collections::HashSet<_> = library.games.iter().map(|g| &g.key).collect();
    assert_eq!(keys.len(), library.games.len());
    let first = &library.games[0];
    assert_eq!(first.key, "steam:1");
    assert_eq!(first.title, "Line break and spaces");
    assert_eq!(first.play_time_ms, 0.);
    assert_eq!(first.last_played, None);
    assert_eq!(first.achievements.unlocked, 10);
    assert!(first.description.chars().count() <= 600);
    assert_eq!(first.art, vec!["cover", "hero", "shot0"]);
    assert!(first.has(crate::console::ArtKind::Shot(0)));
    assert!(!first.has(crate::console::ArtKind::Logo));
    assert!(crate::console::Library::parse(b"not json").is_none());
}

#[test]
fn console_start_path_names_the_game_only_when_safe() {
    use crate::console::start_path;
    assert_eq!(start_path(Some("steam:1313140")), "/play?key=steam:1313140");
    assert_eq!(start_path(None), "/console");
    assert_eq!(start_path(Some("steam:1&kind=x")), "/console");
}

#[test]
fn play_tab_exists_only_with_a_console() {
    let mut state = ready();
    assert_eq!(state.nav_order(), &crate::state::NAV_ORDER);
    state.open(crate::play::PLAY_PAGE);
    assert_eq!(state.page, Page::Home);
    let mut state = play_ready();
    assert!(state.nav_order().contains(&crate::play::PLAY_PAGE));
    state.open(crate::play::PLAY_PAGE);
    assert_eq!(state.page, Page::Play);
    // Explore's tab is next to Play's.
    state.focus = Target::Nav(crate::state::EXPLORE_PAGE);
    state.direction(Direction::Right, &[]);
    assert_eq!(state.page, Page::Play);
}

#[test]
fn play_rail_buttons_and_tab_navigate() {
    let mut state = play_ready();
    state.open(crate::play::PLAY_PAGE);
    state.direction(Direction::Down, &[]);
    assert_eq!(state.focus, Target::PlayGame(0));
    for _ in 0..10 {
        state.direction(Direction::Right, &[]);
    }
    let last = state.play.games().len() - 1;
    assert_eq!(state.focus, Target::PlayGame(last));
    assert_eq!(state.play.index, last);
    state.direction(Direction::Up, &[]);
    assert_eq!(
        state.focus,
        Target::PlayAction(crate::play::PlayButton::Play)
    );
    for _ in 0..4 {
        state.direction(Direction::Right, &[]);
    }
    assert_eq!(
        state.focus,
        Target::PlayAction(crate::play::PlayButton::Hydra)
    );
    state.direction(Direction::Down, &[]);
    assert_eq!(state.focus, Target::PlayGame(last));
    state.back();
    assert_eq!(state.focus, Target::Nav(crate::play::PLAY_PAGE));
    state.back();
    assert_eq!(state.page, Page::Home);
}

#[test]
fn play_backdrop_waits_for_focus_to_rest() {
    let mut state = play_ready();
    state.open(crate::play::PLAY_PAGE);
    state.focus = Target::PlayGame(0);
    state.play_tick();
    let first = state.play.games()[0].key.clone();
    assert_eq!(
        state.play.preview.as_ref().map(|p| p.key.clone()),
        Some(first.clone())
    );
    state.direction(Direction::Right, &[]);
    state.play_tick();
    // Just moved: the backdrop stays.
    assert_eq!(
        state.play.preview.as_ref().map(|p| p.key.clone()),
        Some(first)
    );
    state.play.focused_at =
        Some(std::time::Instant::now() - crate::play::DWELL - std::time::Duration::from_millis(5));
    state.play_tick();
    assert_eq!(
        state.play.preview.as_ref().map(|p| p.key.clone()),
        Some(state.play.games()[1].key.clone())
    );
}

#[test]
fn play_details_open_and_close() {
    let mut state = play_ready();
    state.open(crate::play::PLAY_PAGE);
    state.focus = Target::PlayGame(2);
    state.play_tick();
    state.focus = Target::PlayAction(crate::play::PlayButton::Details);
    assert!(state.activate().is_empty());
    assert_eq!(state.focus, Target::PlayDetailStart);
    assert_eq!(
        state.play.sheet.as_ref().map(|sheet| sheet.key.clone()),
        Some(state.play.games()[2].key.clone())
    );
    state.back();
    assert!(state.play.sheet.is_none());
    assert_eq!(state.focus, Target::PlayGame(2));
}

#[test]
fn play_art_keeps_covers_and_only_the_previewed_games_extras() {
    use crate::console::ArtKind;
    let mut state = play_ready();
    let keys: Vec<_> = state.play.games().iter().map(|g| g.key.clone()).collect();
    let image = || crate::app_artwork::Artwork {
        key: "test".into(),
        pixels: image::RgbaImage::new(1, 1),
        color: [0, 0, 0],
    };
    for key in &keys[..3] {
        for kind in [ArtKind::Cover, ArtKind::Shot(0), ArtKind::Hero] {
            state.set_console_art(key.clone(), kind, Some(image()));
        }
    }
    state.play.preview = Some(crate::play::Preview {
        key: keys[2].clone(),
        since: std::time::Instant::now(),
    });
    state.play.evict(None);
    // Covers stay; the first game's (the band's) and the previewed game's extras stay.
    for key in &keys[..3] {
        assert!(state.play.image(key, ArtKind::Cover).is_some());
    }
    assert!(state.play.image(&keys[0], ArtKind::Hero).is_some());
    assert!(state.play.image(&keys[1], ArtKind::Shot(0)).is_none());
    assert!(state.play.image(&keys[2], ArtKind::Shot(0)).is_some());
    // Art for a game that is not in the library is not kept.
    state.set_console_art("steam:999".into(), ArtKind::Cover, Some(image()));
    assert!(state.play.image("steam:999", ArtKind::Cover).is_none());
}

#[test]
fn play_stats_read_naturally() {
    use crate::i18n::Language;
    use crate::play::{achievement_share, hours, played_ago, size, year};
    let en = Language::En;
    assert_eq!(hours(30. * 60_000., en), "30 min");
    assert_eq!(hours(41.3 * 3_600_000., en), "41.3 h");
    assert_eq!(hours(250. * 3_600_000., en), "250 h");
    assert_eq!(hours(0., en), "Not played yet");
    assert_eq!(size(Some(48.2e9)).as_deref(), Some("48.2 GB"));
    assert_eq!(size(Some(820e6)).as_deref(), Some("820 MB"));
    assert_eq!(year("24 Feb, 2022"), Some("2022"));
    let now = time::OffsetDateTime::parse(
        "2026-11-16T12:00:00Z",
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    assert_eq!(
        played_ago(Some("2026-11-14T12:00:00Z"), now, en).as_deref(),
        Some("Played 2 days ago")
    );
    assert_eq!(
        played_ago(Some("2026-11-16T08:00:00Z"), now, en).as_deref(),
        Some("Played today")
    );
    let game = crate::console::Game {
        achievements: crate::console::Achievements {
            unlocked: 37,
            total: 42,
        },
        ..Default::default()
    };
    assert!((achievement_share(&game).unwrap() - 37. / 42.).abs() < 1e-6);
}

#[test]
fn hiding_a_game_from_its_details_and_bringing_it_back() {
    let mut state = play_ready();
    state.open(crate::play::PLAY_PAGE);
    let count = state.play.games().len();
    let key = state.play.games()[1].key.clone();
    state.focus = Target::PlayGame(1);
    // Holding OK on a cover opens its details; holding again there hides it.
    assert_eq!(state.card_hold(), Some(vec![]));
    assert_eq!(state.focus, Target::PlayDetailStart);
    assert!(matches!(state.card_hold().as_deref(), Some([Effect::Save])));
    assert!(state.play.sheet.is_none());
    assert_eq!(state.preferences.hidden_games, vec![key.clone()]);
    assert_eq!(state.play.games().len(), count - 1);
    assert!(state.play.games().iter().all(|game| game.key != key));
    assert_eq!(state.focus, Target::PlayGame(1));
    // A new library from the PC keeps it hidden.
    state.set_console_library(crate::console::Library::default());
    state.set_console_library(crate::console::demo_library());
    assert_eq!(state.play.games().len(), count - 1);
    // The rail simply ends: no tile for the hidden games.
    state.focus = Target::PlayGame(count - 2);
    state.direction(Direction::Right, &[]);
    assert_eq!(state.focus, Target::PlayGame(count - 2));
    // Settings > Show hidden games brings it back, marked.
    show_hidden_games(&mut state);
    assert!(state.preferences.show_hidden_games);
    assert_eq!(state.play.games().len(), count);
    state.open(crate::play::PLAY_PAGE);
    state.focus = Target::PlayGame(1);
    state.play_tick();
    // Its details offer Unhide; the Hide button sits under Play.
    state.focus = Target::PlayAction(crate::play::PlayButton::Details);
    state.activate();
    state.direction(Direction::Down, &[]);
    assert_eq!(state.focus, Target::PlayHide);
    assert!(matches!(state.activate()[..], [Effect::Save]));
    assert!(state.preferences.hidden_games.is_empty());
    // The setting stays as it was.
    assert!(state.preferences.show_hidden_games);
    assert_eq!(state.play.games().len(), count);
}

fn show_hidden_games(state: &mut State) {
    state.open(2);
    state.focus = Target::Setting(crate::state::SHOW_HIDDEN_GAMES_SETTING);
    assert!(matches!(state.activate()[..], [Effect::Save]));
}

#[test]
fn the_show_hidden_games_setting_refilters_the_rail() {
    let mut state = play_ready();
    let count = state.play.games().len();
    state.preferences.hidden_games = vec![state.play.games()[0].key.clone()];
    state.set_console_library(crate::console::Library::default());
    state.set_console_library(crate::console::demo_library());
    assert_eq!(state.play.games().len(), count - 1);
    // Its row is in Settings with the console set up.
    state.open(2);
    assert!(
        controls(&state)
            .iter()
            .any(|c| c.target == Target::Setting(crate::state::SHOW_HIDDEN_GAMES_SETTING))
    );
    show_hidden_games(&mut state);
    assert_eq!(state.play.games().len(), count);
    assert!(state.play.is_hidden(&state.play.games()[0].key));
    show_hidden_games(&mut state);
    assert!(!state.preferences.show_hidden_games);
    assert_eq!(state.play.games().len(), count - 1);
    // Every game hidden: the page offers Open Hydra.
    state.preferences.hidden_games = state
        .play
        .all()
        .unwrap()
        .games
        .iter()
        .map(|game| game.key.clone())
        .collect();
    state.set_console_library(crate::console::Library::default());
    state.set_console_library(crate::console::demo_library());
    assert!(state.play.games().is_empty());
    assert_eq!(
        state.play_entry(),
        Target::PlayAction(crate::play::PlayButton::Hydra)
    );
}

#[test]
fn picking_a_game_launches_it_from_its_card() {
    let mut state = play_ready();
    state.open(crate::play::PLAY_PAGE);
    state.focus = Target::PlayGame(1);
    let effects = state.activate();
    assert!(matches!(effects[..], [Effect::ConsoleStart(_)]));
    let session = state.console.as_ref().unwrap();
    assert_eq!(session.source, Target::PlayGame(1));
    assert_eq!(
        session.game.as_ref().map(|g| g.key.clone()),
        Some(state.play.games()[1].key.clone())
    );
    assert_eq!(state.sound, Some(crate::state::Sound::Launch));
    // Open Hydra launches no game; the Play button grows from its cover.
    let mut state = play_ready();
    state.open(crate::play::PLAY_PAGE);
    state.focus = Target::PlayAction(crate::play::PlayButton::Hydra);
    state.activate();
    assert!(state.console.as_ref().unwrap().game.is_none());
    let mut state = play_ready();
    state.open(crate::play::PLAY_PAGE);
    state.play.index = 3;
    state.focus = Target::PlayAction(crate::play::PlayButton::Play);
    state.activate();
    assert_eq!(state.console.as_ref().unwrap().source, Target::PlayGame(3));
}

#[test]
fn launch_cues_play_once_each() {
    use crate::console::{MIN_SHOW, PULSE, Phase, READY_HOLD, Session};
    use crate::state::Sound;
    let start = std::time::Instant::now();
    let mut session = Session::new(1, Target::Nav(0), Target::Nav(0), None, start);
    assert_eq!(session.cue(start), None);
    assert_eq!(session.cue(start + PULSE / 2), Some(Sound::Waiting));
    assert_eq!(session.cue(start + PULSE / 2), None);
    session.update(Phase::Ready, start + PULSE);
    let ready = start + PULSE.max(MIN_SHOW);
    assert_eq!(session.cue(ready), Some(Sound::Ready));
    // The chime carries the whoosh: nothing more.
    assert_eq!(session.cue(ready + READY_HOLD), None);
    let mut forced = Session::new(1, Target::Nav(0), Target::Nav(0), None, start);
    forced.force(start + PULSE);
    assert_eq!(forced.cue(start + PULSE), Some(Sound::Whoosh));
    assert_eq!(forced.cue(start + PULSE), None);
}

#[test]
fn reduced_motion_launch_has_no_burst_ring() {
    // The burst ring: a wide border round the orbit's centre, a moment after Ready.
    let rings = |reduced: bool| {
        let mut state = play_ready();
        state.preferences.reduced_motion = reduced;
        state.open(crate::play::PLAY_PAGE);
        state.focus = Target::PlayGame(0);
        state.activate();
        let session = state.console.as_mut().unwrap();
        session.update(crate::console::Phase::Ready, std::time::Instant::now());
        session.ready_at = Some(std::time::Instant::now() - std::time::Duration::from_millis(300));
        scene(&state)
            .draw
            .iter()
            .filter(|draw| {
                matches!(draw, crate::renderer::Draw::Shape(rect, _, border, _)
                    if *border > 0.
                        && rect.w > 300.
                        && (rect.center().0 - 960.).abs() < 2.
                        && (rect.center().1 - 600.).abs() < 2.)
            })
            .count()
    };
    assert!(rings(false) > 0);
    assert_eq!(rings(true), 0);
}

#[test]
fn play_backdrop_keeps_moving_without_input() {
    let mut state = play_ready();
    state.open(crate::play::PLAY_PAGE);
    state.tick();
    state.dirty = false;
    // No key: the next frame is still drawn, as the screenshots drift.
    state.tick();
    assert!(state.dirty);
    assert!(state.animating());
    // With Reduce motion it is drawn only when the screenshot changes.
    state.preferences.reduced_motion = true;
    state.tick();
    state.dirty = false;
    state.tick();
    assert!(!state.dirty);
}

// ---- Photos from the PC ----

fn photo(width: u32, height: u32, id: u64) -> crate::photos::Photo {
    let image = image::RgbImage::from_pixel(width, height, image::Rgb([90, 120, 150]));
    let mut bytes = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    crate::photos::Photo::decode(&bytes, id).unwrap()
}

fn photos_ready() -> State {
    let mut state = console_ready();
    state.console_awake = Some(true);
    state
}

#[test]
fn a_portrait_photo_stands_over_a_blurred_copy() {
    let portrait = photo(300, 600, 1);
    assert_eq!(portrait.image.key, "photo:1");
    let fill = portrait.fill.as_ref().unwrap();
    assert_eq!(fill.key, "backdrop:photo-fill:1");
    assert_eq!(fill.pixels.dimensions(), crate::photos::FILL);
    let landscape = photo(600, 300, 2);
    assert_eq!(landscape.image.key, "backdrop:photo:2");
    assert!(landscape.fill.is_none());
}

#[test]
fn photos_are_asked_for_only_while_the_pc_answers_and_they_are_on() {
    let mut state = console_ready();
    assert!(!state.photo_wanted());
    state.console_awake = Some(false);
    assert!(!state.photo_wanted());
    state.console_awake = Some(true);
    assert!(state.photo_wanted());
    state.photos.request();
    assert!(!state.photo_wanted());
    state.photo_received(Some(photo(64, 36, 1)));
    // One is ready: no more until it is shown.
    assert!(!state.photo_wanted());
    let mut off = photos_ready();
    off.preferences.photo_screensaver = false;
    off.preferences.photo_background = false;
    assert!(!off.photo_wanted());
    // Without the console set up, never.
    let mut plain = State::new(Preferences::default());
    plain.console_awake = Some(true);
    assert!(!plain.photo_wanted());
}

#[test]
fn the_screensaver_shows_photos_when_the_pc_has_them() {
    use std::time::Duration;
    let idle = Duration::from_secs(60 * 60);
    // No feeds and no photo: no screensaver.
    let mut state = photos_ready();
    state.ambient_tick(idle, true);
    assert!(state.feeds.ambient.is_none());
    state.photos.request();
    state.photo_received(Some(photo(64, 36, 1)));
    state.ambient_tick(idle, true);
    let ambient = state.feeds.ambient.as_ref().unwrap();
    assert!(ambient.photos);
    assert!(state.photos.current.is_some());
    // OK only ends it: a photo has no title page.
    state.ambient_input(true);
    assert!(state.feeds.ambient.is_none());
    assert!(state.feeds.title.is_none());
    // Turned off, the photo is not used.
    let mut off = photos_ready();
    off.preferences.photo_screensaver = false;
    off.photos.request();
    off.photo_received(Some(photo(64, 36, 1)));
    off.ambient_tick(idle, true);
    assert!(off.feeds.ambient.is_none());
}

#[test]
fn photos_stop_when_the_pc_stops_answering() {
    let mut state = photos_ready();
    state.photos.request();
    state.photo_received(Some(photo(64, 36, 1)));
    state.photos_tick(true);
    assert!(state.home_photo());
    for _ in 0..2 {
        state.photos.request();
        state.photo_received(None);
    }
    assert!(!state.photos_live());
    assert!(!state.home_photo());
    assert!(!state.photo_wanted());
    // The PC answering a probe again tries at once.
    state.photos.reset_failures();
    assert!(state.home_photo());
}

#[test]
fn home_photos_change_slowly_and_hold_at_most_three() {
    use crate::photos::{FADE, HOME_SLIDE, Photos};
    use std::time::Instant;
    let mut photos = Photos::default();
    let start = Instant::now();
    photos.received(Some(photo(64, 36, 1)));
    // The first shows at once, over the wallpaper.
    assert!(photos.tick(HOME_SLIDE, start));
    assert!(photos.previous.is_none());
    photos.received(Some(photo(64, 36, 2)));
    assert!(!photos.tick(HOME_SLIDE, start + HOME_SLIDE / 2));
    assert!(photos.tick(HOME_SLIDE, start + HOME_SLIDE));
    assert_eq!(
        photos.current.as_ref().unwrap().0.image.key,
        "backdrop:photo:2"
    );
    assert!(photos.previous.is_some() && photos.next.is_none());
    // The one before is let go once the new one has faded in.
    assert!(photos.tick(HOME_SLIDE, start + HOME_SLIDE + FADE));
    assert!(photos.previous.is_none());
}

#[test]
fn photo_settings_toggle() {
    use crate::state::{PHOTO_BACKGROUND_SETTING, PHOTO_SCREENSAVER_SETTING};
    let mut state = photos_ready();
    state.open(2);
    state.focus = Target::Setting(PHOTO_SCREENSAVER_SETTING);
    state.activate();
    assert!(!state.preferences.photo_screensaver);
    state.focus = Target::Setting(PHOTO_BACKGROUND_SETTING);
    state.activate();
    assert!(!state.preferences.photo_background);
}

// ---- The PC's controllers and drives, and deleting games ----

#[test]
fn console_system_is_bounded_and_cleaned() {
    use crate::console::System;
    let pads: Vec<_> = (0..12)
        .map(|n| json!({"name": format!("Pad\n{n}"), "battery": 250, "charging": n == 0}))
        .collect();
    let reply = json!({
        "controllers": pads,
        "drives": [
            {"letter": "c", "label": "Sys\ttem", "free": 2e12, "total": 1e12},
            {"letter": "D:", "label": "", "free": -5.0, "total": 5e11},
            {"letter": "E", "label": "", "free": 1e9, "total": 0.0},
        ],
    });
    let system = System::parse(reply.to_string().as_bytes()).unwrap();
    assert_eq!(system.controllers.len(), crate::console::SYSTEM_ITEMS);
    assert_eq!(system.controllers[0].name, "Pad 0");
    assert!(system.controllers.iter().all(|pad| pad.battery == 100));
    // A drive without a size is left out; free space stays within the drive.
    assert_eq!(system.drives.len(), 2);
    assert_eq!(system.drives[0].letter, "C");
    assert_eq!(system.drives[0].label, "Sys tem");
    assert_eq!(system.drives[0].free, 1e12);
    assert_eq!(system.drives[1].letter, "D");
    assert_eq!(system.drives[1].free, 0.);
    assert!((system.drives[1].used() - 1.).abs() < 1e-6);
    assert!(System::parse(b"not json").is_none());
    assert!(System::parse(br#"{"controllers":[{"battery":-3}]}"#).is_none());
}

#[test]
fn console_delete_jobs_parse() {
    use crate::console::{DeleteError, JobState, valid_job_id};
    assert!(valid_job_id("0f8b6c2e-1d3a-4b5c-9e7f-0123456789ab"));
    assert!(!valid_job_id("0F8B6C2E-1D3A-4B5C-9E7F-0123456789AB"));
    assert!(!valid_job_id("../../etc/passwd"));
    assert_eq!(
        JobState::parse(br#"{"state":"pending"}"#),
        Some(JobState::Pending)
    );
    assert_eq!(
        JobState::parse(br#"{"state":"done"}"#),
        Some(JobState::Done)
    );
    assert_eq!(
        JobState::parse(br#"{"state":"failed","error":"running"}"#),
        Some(JobState::Failed(DeleteError::Running))
    );
    assert_eq!(
        JobState::parse(br#"{"state":"failed","error":"io"}"#),
        Some(JobState::Failed(DeleteError::Failed))
    );
    assert_eq!(JobState::parse(br#"{"state":"odd"}"#), None);
}

fn play_sheet() -> State {
    let mut state = play_ready();
    state.console_awake = Some(true);
    state.open(crate::play::PLAY_PAGE);
    state.focus = Target::PlayGame(1);
    state.play_tick();
    state.focus = Target::PlayAction(crate::play::PlayButton::Details);
    state.activate();
    state
}

#[test]
fn the_details_sheet_reaches_delete_and_back() {
    let mut state = play_sheet();
    assert_eq!(state.focus, Target::PlayDetailStart);
    state.direction(Direction::Down, &[]);
    assert_eq!(state.focus, Target::PlayHide);
    state.direction(Direction::Down, &[]);
    assert_eq!(state.focus, Target::PlayDelete);
    assert!(
        controls(&state)
            .iter()
            .any(|c| c.target == Target::PlayDelete)
    );
    state.direction(Direction::Down, &[]);
    assert_eq!(state.focus, Target::PlayDelete);
    state.direction(Direction::Up, &[]);
    assert_eq!(state.focus, Target::PlayHide);
    state.direction(Direction::Up, &[]);
    assert_eq!(state.focus, Target::PlayDetailStart);
}

#[test]
fn deleting_asks_first_and_sends_the_choice() {
    use crate::play::{ConfirmPhase, DeleteChoice};
    for (moves, keep) in [(0, true), (1, false)] {
        let mut state = play_sheet();
        let key = state.play.sheet.as_ref().unwrap().key.clone();
        state.focus = Target::PlayDelete;
        assert!(state.activate().is_empty());
        assert_eq!(state.focus, Target::PlayConfirm(DeleteChoice::KeepSaves));
        // The sheet's own controls cannot be reached behind it.
        assert!(!controls(&state).iter().any(|c| matches!(
            c.target,
            Target::PlayDetailStart | Target::PlayHide | Target::PlayDelete | Target::PlayShot(_)
        )));
        assert!(
            controls(&state)
                .iter()
                .any(|c| c.target == Target::PlayConfirm(DeleteChoice::AllData))
        );
        for _ in 0..moves {
            state.direction(Direction::Down, &[]);
        }
        let effects = state.activate();
        assert_eq!(
            effects,
            vec![Effect::ConsoleDelete {
                key: key.clone(),
                keep_saves: keep,
            }]
        );
        let phase = state
            .play
            .sheet
            .as_ref()
            .unwrap()
            .confirm
            .as_ref()
            .unwrap()
            .phase;
        assert_eq!(phase, ConfirmPhase::Deleting);
        // While it runs, OK and the arrows do nothing.
        assert!(state.activate().is_empty());
        state.direction(Direction::Up, &[]);
        assert!(state.play.sheet.as_ref().unwrap().confirm.is_some());
        // Done: the game leaves, the sheet closes.
        let count = state.play.games().len();
        state.console_deleted(&key, Ok(()));
        assert!(state.play.sheet.is_none());
        assert_eq!(state.play.games().len(), count - 1);
        assert!(state.play.find(&key).is_none());
        assert!(state.play.deleting.is_none());
    }
}

#[test]
fn delete_cancel_back_and_failure() {
    use crate::console::DeleteError;
    use crate::play::{ConfirmPhase, DeleteChoice};
    let mut state = play_sheet();
    state.focus = Target::PlayDelete;
    state.activate();
    // Cancel, at the bottom, closes it.
    state.direction(Direction::Down, &[]);
    state.direction(Direction::Down, &[]);
    state.direction(Direction::Down, &[]);
    assert_eq!(state.focus, Target::PlayConfirm(DeleteChoice::Cancel));
    assert!(state.activate().is_empty());
    assert!(state.play.sheet.as_ref().unwrap().confirm.is_none());
    assert_eq!(state.focus, Target::PlayDelete);
    // Back closes it too, leaving the sheet open.
    state.activate();
    state.back();
    assert!(state.play.sheet.as_ref().unwrap().confirm.is_none());
    assert!(state.play.sheet.is_some());
    assert_eq!(state.focus, Target::PlayDelete);
    // A failure says why.
    let key = state.play.sheet.as_ref().unwrap().key.clone();
    state.activate();
    assert_eq!(state.activate().len(), 1);
    state.console_deleted(&key, Err(DeleteError::Running));
    let confirm = state.play.sheet.as_ref().unwrap().confirm.as_ref().unwrap();
    assert_eq!(confirm.phase, ConfirmPhase::Failed(DeleteError::Running));
    assert!(state.play.find(&key).is_some());
    assert_eq!(state.focus, Target::PlayConfirm(DeleteChoice::Cancel));
    state.activate();
    assert!(state.play.sheet.as_ref().unwrap().confirm.is_none());
}

#[test]
fn delete_waits_for_the_pc() {
    let mut state = play_sheet();
    state.console_awake = Some(false);
    state.focus = Target::PlayDelete;
    assert!(state.activate().is_empty());
    assert!(state.play.sheet.as_ref().unwrap().confirm.is_none());
    let game = state
        .play
        .find(&state.play.sheet.as_ref().unwrap().key)
        .unwrap();
    assert_eq!(state.delete_blocked(game), Some("deleteAsleep"));
    // A running game cannot be deleted either.
    state.console_awake = Some(true);
    let key = state.play.sheet.as_ref().unwrap().key.clone();
    let mut library = state.play.all().unwrap().clone();
    for game in &mut library.games {
        game.running = game.key == key;
    }
    state.set_console_library(library);
    state.focus = Target::PlayDelete;
    assert!(state.activate().is_empty());
    assert!(state.play.sheet.as_ref().unwrap().confirm.is_none());
}

#[test]
fn the_status_strip_shows_while_the_pc_answers() {
    let mut state = play_ready();
    state.open(crate::play::PLAY_PAGE);
    for awake in [false, true] {
        state.console_awake = Some(awake);
        state.play.system = None;
        let without = scene(&state).draw.len();
        state.play.system = Some(crate::console::demo_system());
        let with = scene(&state).draw.len();
        assert_eq!(with > without, awake);
    }
}

// ---- Browse by Platform ----

const PLATFORM_SOURCE: &str = "platforms:netflix,disney-plus,hbo-max,prime-video,apple-tv,paramount-plus,crunchyroll,stan,binge";

#[test]
fn platform_sources_parse_and_skip_unknown_keys() {
    use crate::feeds::platforms::{PLATFORMS, from_link, is_source, items, parse};
    assert!(is_source(" platforms:netflix"));
    assert!(!is_source("mdblist:netflix"));
    let stan = PLATFORMS.iter().position(|p| p.key == "stan").unwrap();
    assert_eq!(
        parse("platforms:netflix, hulu ,stan,netflix,,"),
        vec![0, stan]
    );
    assert!(parse("platforms:").is_empty());
    let cards = items("platforms:netflix,stan");
    assert_eq!(cards.len(), 2);
    assert_eq!(cards[1].title, "Stan");
    assert_eq!(from_link(&cards[1].link), Some(stan));
    assert_eq!(from_link("nuvio://tmdb/movie/603"), None);
    // The full row: one card per platform.
    assert_eq!(items(PLATFORM_SOURCE).len(), PLATFORMS.len());
}

fn platform_home() -> State {
    let mut state = ready();
    state.feeds.can_search = true;
    let row = |title: &str, source: &str, items: Vec<crate::feeds::Item>| crate::feeds::Row {
        title: title.into(),
        pinned: false,
        hero: false,
        extra: false,
        source: source.into(),
        hidden: false,
        status: crate::feeds::Status::Ready,
        items,
    };
    let poster = crate::feeds::Item {
        title: "Dune".into(),
        subtitle: String::new(),
        link: "nuvio://tmdb/movie/438631".into(),
        image: None,
        progress: None,
        episode: None,
        badge: None,
    };
    state.set_feed_rows(vec![
        row("Trending", "mdblist:a/b", vec![poster]),
        row(
            "Browse by Platform",
            PLATFORM_SOURCE,
            crate::feeds::platforms::items(PLATFORM_SOURCE),
        ),
    ]);
    state
}

fn found(titles: &[(&str, &str)]) -> Vec<crate::feeds::explore::Found> {
    titles
        .iter()
        .map(|(title, link)| crate::feeds::explore::Found {
            item: crate::feeds::Item {
                title: (*title).into(),
                link: (*link).into(),
                ..crate::feeds::Item::default()
            },
            year: None,
            rating: None,
        })
        .collect()
}

/// The Discover search the platform page sent last.
fn platform_asked(state: &mut State) -> Option<(u64, crate::feeds::explore::Query)> {
    state.platform_tick();
    state
        .feeds
        .outbox
        .drain(..)
        .filter_map(|out| match out {
            crate::feeds::Outgoing::Discover(id, query) => Some((id, query)),
            _ => None,
        })
        .next_back()
}

#[test]
fn a_platform_tile_opens_its_page_and_back_returns() {
    use crate::feeds::explore::Kind;
    let mut state = platform_home();
    assert!(state.feeds.platform_row(1) && !state.feeds.platform_row(0));
    // The row's tiles are wide; it has no See all tile.
    assert_eq!(
        state
            .feeds
            .card_width(&Target::Feed(1, 0), state.preferences.reduced_motion),
        crate::feeds::PLATFORM_WIDTH
    );
    assert!(
        !controls(&state)
            .iter()
            .any(|c| c.target == Target::FeedMore(1))
    );
    let last = crate::feeds::platforms::PLATFORMS.len() - 1;
    state.focus = Target::Feed(1, last);
    state.direction(Direction::Right, &controls(&state));
    assert_eq!(state.focus, Target::Feed(1, last));
    // OK on Netflix's tile: its page over Home, on its movies.
    state.focus = Target::Feed(1, 0);
    assert!(state.activate().is_empty());
    assert_eq!(state.page, Page::Home);
    let page = state.feeds.platform.as_ref().unwrap();
    assert_eq!((page.platform, page.kind), (0, Kind::Movies));
    assert_eq!(state.focus, Target::PlatformItem(0));
    // One search for its movies, then none while it is looked for.
    let (id, query) = platform_asked(&mut state).unwrap();
    assert_eq!(
        (query.kind, query.provider, query.genre),
        (Kind::Movies, Some(0), None)
    );
    assert!(platform_asked(&mut state).is_none());
    // Still loading: the pills hold focus.
    state.direction(Direction::Down, &[]);
    assert_eq!(state.focus, Target::PlatformKind(0));
    state.discover_arrived(
        id,
        query,
        Ok(found(&[
            ("Dune", "nuvio://tmdb/movie/438631"),
            ("Heat", "nuvio://tmdb/movie/949"),
        ])),
    );
    state.platform_discovered(id, None);
    assert!(
        controls(&state)
            .iter()
            .any(|c| c.target == Target::PlatformPlay)
    );
    // Along the row, the title shown large follows.
    state.direction(Direction::Down, &[]);
    assert_eq!(state.focus, Target::PlatformPlay);
    state.direction(Direction::Down, &[]);
    state.direction(Direction::Right, &[]);
    state.direction(Direction::Right, &[]);
    assert_eq!(state.focus, Target::PlatformItem(1));
    assert_eq!(state.feeds.platform_hero().unwrap().title, "Heat");
    // Play plays it.
    state.direction(Direction::Up, &[]);
    assert_eq!(state.focus, Target::PlatformPlay);
    assert!(matches!(
        state.activate().as_slice(),
        [Effect::LaunchTitle(item)] if item.title == "Heat"
    ));
    // Details opens its page over this one; Back comes back here.
    state.focus = Target::PlatformDetails;
    state.activate();
    assert_eq!(state.feeds.title.as_ref().unwrap().item.title, "Heat");
    state.back();
    assert!(state.feeds.title.is_none() && state.feeds.platform.is_some());
    assert_eq!(state.focus, Target::PlatformDetails);
    // Back again: Home, on the tile.
    state.back();
    assert!(state.feeds.platform.is_none());
    assert_eq!(state.focus, Target::Feed(1, 0));
}

#[test]
fn the_platform_pills_switch_to_shows() {
    use crate::feeds::explore::Kind;
    let mut state = platform_home();
    state.focus = Target::Feed(1, 7);
    state.activate();
    let (id, query) = platform_asked(&mut state).unwrap();
    state.discover_arrived(
        id,
        query,
        Ok(found(&[("Bluey", "nuvio://tmdb/series/82728")])),
    );
    state.focus = Target::PlatformKind(0);
    state.direction(Direction::Right, &[]);
    assert_eq!(state.focus, Target::PlatformKind(1));
    state.activate();
    let page = state.feeds.platform.as_ref().unwrap();
    assert_eq!((page.kind, page.hero), (Kind::Shows, 0));
    let (_, shows) = platform_asked(&mut state).unwrap();
    assert_eq!((shows.kind, shows.provider), (Kind::Shows, Some(7)));
    // Nothing found yet for shows: an empty row.
    assert!(state.feeds.platform_items().is_empty());
    // A failed search says so, and is not asked again.
    let (id, _) = state.feeds.platform.as_ref().unwrap().asked.unwrap();
    state.platform_discovered(id, Some("offline".into()));
    assert!(platform_asked(&mut state).is_none());
}

#[test]
fn a_platform_tile_needs_tmdb() {
    let mut state = platform_home();
    state.feeds.can_search = false;
    state.focus = Target::Feed(1, 0);
    state.activate();
    assert!(state.feeds.platform.is_none());
}

#[test]
fn title_pages_list_directors_and_writers_first() {
    let movie = serde_json::json!({"credits": {"crew": [
        {"id": 1, "name": "Denis Villeneuve", "job": "Director", "profile_path": "/d.jpg"},
        {"id": 1, "name": "Denis Villeneuve", "job": "Screenplay"},
        {"id": 2, "name": "Jon Spaihts", "job": "Screenplay"},
        {"id": 3, "name": "Frank Herbert", "job": "Novel"},
        {"id": 4, "name": "Hans Zimmer", "job": "Original Music Composer"},
        {"id": 5, "name": "Eric Roth", "job": "Writer"},
        {"id": 6, "name": "Someone Else", "job": "Story"}
    ]}});
    let crew = crate::feeds_service::title_crew(false, &movie);
    let names: Vec<(&str, &str)> = crew
        .iter()
        .map(|person| (person.name.as_str(), person.role.as_str()))
        .collect();
    assert_eq!(
        names,
        vec![
            ("Denis Villeneuve", "Director, Writer"),
            ("Jon Spaihts", "Writer"),
            ("Eric Roth", "Writer"),
            ("Someone Else", "Story"),
        ]
    );
    assert!(crew.iter().all(|person| person.crew));
    assert_eq!(
        crew[0].image.as_deref(),
        Some("https://image.tmdb.org/t/p/w185/d.jpg")
    );
    let show = serde_json::json!({"created_by": [{"id": 9, "name": "Vince Gilligan"}]});
    let crew = crate::feeds_service::title_crew(true, &show);
    assert_eq!((crew[0].id, crew[0].role.as_str()), (9, "Creator"));
}

#[test]
fn a_person_page_groups_their_work() {
    let person = serde_json::json!({
        "name": "Greta Gerwig",
        "known_for_department": "Directing",
        "birthday": "1983-08-04",
        "place_of_birth": "Sacramento, California, USA",
        "biography": "An American filmmaker.",
        "profile_path": "/g.jpg",
        "combined_credits": {
            "cast": [
                {"id": 10, "media_type": "movie", "title": "Frances Ha", "vote_count": 900, "character": "Frances"},
                {"id": 11, "media_type": "tv", "name": "Late Show", "vote_count": 5000, "genre_ids": [10767], "character": "Guest"},
                {"id": 12, "media_type": "movie", "title": "Oscars", "vote_count": 10, "character": "Herself"},
                {"id": 13, "media_type": "tv", "name": "Self Help", "vote_count": 10, "character": "Self - Host"}
            ],
            "crew": [
                {"id": 20, "media_type": "movie", "title": "Lady Bird", "vote_count": 8000, "job": "Director", "department": "Directing"},
                {"id": 20, "media_type": "movie", "title": "Lady Bird", "vote_count": 8000, "job": "Writer", "department": "Writing"},
                {"id": 21, "media_type": "movie", "title": "Barbie", "vote_count": 9000, "job": "Director", "department": "Directing", "poster_path": "/b.jpg"},
                {"id": 21, "media_type": "movie", "title": "Barbie", "vote_count": 9000, "job": "Director", "department": "Directing"}
            ]
        }
    });
    let info = crate::feeds_service::person_from(&person);
    assert_eq!(info.name, "Greta Gerwig");
    assert_eq!(info.born, "Born 1983  ·  Sacramento, California, USA");
    let rows: Vec<(&str, Vec<&str>)> = info
        .rows
        .iter()
        .map(|(name, items)| {
            (
                name.as_str(),
                items.iter().map(|item| item.title.as_str()).collect(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("Directing", vec!["Barbie", "Lady Bird"]),
            ("Writing", vec!["Lady Bird"]),
            ("Acting", vec!["Frances Ha"]),
        ]
    );
    assert_eq!(info.rows[0].1[0].link, "nuvio://tmdb/movie/21");
}

fn person_title(state: &mut State) {
    let item = crate::feeds::Item {
        title: "Dune".into(),
        link: "nuvio://tmdb/movie/438631".into(),
        ..crate::feeds::Item::default()
    };
    state.open_title(item, Target::Feed(0, 0));
    let info = crate::feeds::TitleInfo {
        cast: vec![crate::feeds::Person {
            id: 7,
            name: "Denis Villeneuve".into(),
            role: "Director".into(),
            image: None,
            crew: true,
        }],
        ..crate::feeds::TitleInfo::default()
    };
    state
        .feeds
        .title_arrived("nuvio://tmdb/movie/438631".into(), Ok(info));
    state.feeds.outbox.clear();
}

#[test]
fn a_cast_and_crew_photo_opens_their_page_and_back_unwinds() {
    let mut state = platform_home();
    person_title(&mut state);
    assert!(scene(&state).draw.iter().any(|draw| matches!(
        draw,
        crate::renderer::Draw::Text(words, ..) if words == "Cast & Crew"
    )));
    state.focus = Target::TitleCast(0);
    state.activate();
    assert!(state.feeds.title.is_none());
    assert_eq!(state.feeds.person.as_ref().unwrap().person.id, 7);
    assert!(
        state
            .feeds
            .outbox
            .contains(&crate::feeds::Outgoing::Person(7))
    );
    let rows = vec![
        (
            "Directing".to_owned(),
            vec![
                crate::feeds::Item {
                    title: "Arrival".into(),
                    link: "nuvio://tmdb/movie/329865".into(),
                    ..crate::feeds::Item::default()
                },
                crate::feeds::Item {
                    title: "Sicario".into(),
                    link: "nuvio://tmdb/movie/273481".into(),
                    ..crate::feeds::Item::default()
                },
            ],
        ),
        (
            "Writing".to_owned(),
            vec![crate::feeds::Item {
                title: "Incendies".into(),
                link: "nuvio://tmdb/movie/46738".into(),
                ..crate::feeds::Item::default()
            }],
        ),
    ];
    state.person_arrived(
        7,
        Ok(crate::feeds::PersonInfo {
            name: "Denis Villeneuve".into(),
            rows,
            ..crate::feeds::PersonInfo::default()
        }),
    );
    assert!(
        controls(&state)
            .iter()
            .any(|c| c.target == Target::PersonCredit(1, 0))
    );
    state.direction(Direction::Right, &[]);
    assert_eq!(state.focus, Target::PersonCredit(0, 1));
    state.direction(Direction::Down, &[]);
    assert_eq!(state.focus, Target::PersonCredit(1, 0));
    state.direction(Direction::Up, &[]);
    state.direction(Direction::Right, &[]);
    assert_eq!(state.focus, Target::PersonCredit(0, 1));
    // OK on Sicario: its page; Back: the person, on Sicario; Back: Dune; Back: Home.
    state.activate();
    assert_eq!(state.feeds.title.as_ref().unwrap().item.title, "Sicario");
    assert!(state.feeds.person.is_none());
    state.back();
    assert_eq!(state.feeds.person.as_ref().unwrap().person.id, 7);
    assert_eq!(state.focus, Target::PersonCredit(0, 1));
    state.back();
    assert_eq!(state.feeds.title.as_ref().unwrap().item.title, "Dune");
    state.back();
    assert!(state.feeds.title.is_none() && state.feeds.person.is_none());
    assert_eq!(state.focus, Target::Feed(0, 0));
}

#[test]
fn discover_asks_for_the_service_in_its_region() {
    use crate::feeds::explore::Query;
    use crate::feeds::platforms::PLATFORMS;
    let stan = PLATFORMS.iter().position(|p| p.key == "stan").unwrap();
    let params = crate::feeds_service::provider_params(&Query {
        provider: Some(stan),
        ..Query::default()
    });
    let get = |name: &str| {
        params
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    };
    assert_eq!(get("with_watch_providers"), Some("21"));
    assert_eq!(get("watch_region"), Some("AU"));
    assert_eq!(
        get("with_watch_monetization_types"),
        Some("flatrate|free|ads")
    );
    assert!(crate::feeds_service::provider_params(&Query::default()).is_empty());
    let paramount = PLATFORMS
        .iter()
        .find(|p| p.key == "paramount-plus")
        .unwrap();
    assert_eq!(paramount.providers, "2303|2616");
}
