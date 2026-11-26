//! The application settings' defaults are the header's numbers, and they have to
//! stay that way: `set_application_settings` only overwrites what a script wires,
//! so every colour and unit a script leaves alone comes from here.

use engine_sim_script::objects::ApplicationSettings;

#[test]
fn the_defaults_are_the_ones_the_header_declares() {
    let settings = ApplicationSettings::default();

    assert!(!settings.start_fullscreen);
    assert_eq!(settings.power_units, "hp");
    assert_eq!(settings.torque_units, "lb-ft");
    assert_eq!(settings.speed_units, "mph");
    assert_eq!(settings.pressure_units, "inHg");
    assert_eq!(settings.boost_units, "psi");

    assert_eq!(settings.color_background, 0x0E1012);
    assert_eq!(settings.color_foreground, 0xFFFFFF);
    assert_eq!(settings.color_shadow, 0x0E1012);
    assert_eq!(settings.color_highlight1, 0xEF4545);
    assert_eq!(settings.color_highlight2, 0xFFFFFF);
    assert_eq!(settings.color_pink, 0xF394BE);
    assert_eq!(settings.color_red, 0xEE4445);
    assert_eq!(settings.color_orange, 0xF4802A);
    assert_eq!(settings.color_yellow, 0xFDBD2E);
    assert_eq!(settings.color_blue, 0x77CEE0);
    assert_eq!(settings.color_green, 0xBDD869);
}

#[test]
fn set_color_names_the_field_it_is_given() {
    let mut settings = ApplicationSettings::default();
    settings.set_color("color_pink", 1);
    settings.set_color("color_green", 2);
    settings.set_color("not_a_colour", 3);

    assert_eq!(settings.color_pink, 1);
    assert_eq!(settings.color_green, 2);
    assert_eq!(settings.color_red, 0xEE4445);
}
