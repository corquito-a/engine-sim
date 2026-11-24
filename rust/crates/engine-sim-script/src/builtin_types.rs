//! Built-in node type -> channel type, as `es_script::LanguageRules` registers
//! it.
//!
//! `registerBuiltinType<T>("__engine_sim__float", &piranha::FundamentalType::FloatType)`
//! makes `LanguageRules::resolveChannelType("__engine_sim__float")` return that
//! `ChannelType`, and `IrAttributeDefinition::getImmediateChannelType` reports
//! its name for a port whose type definition is a builtin. The names come from
//! `piranha/src/fundamental_types.cpp` (`FloatNodeType`, ...) and from
//! `scripting/src/channel_types.cpp`, where `DEFINE_CHANNEL(X)` names the type
//! after the identifier (`EngineChannel`).
//!
//! Builtins registered without a channel type have no entry here: their
//! `BuiltinTypeInfo::nodeType` is null, so the original reports no type either.

/// `LanguageRules::resolveChannelType`
pub fn builtin_channel_type(builtin_name: &str) -> Option<&'static str> {
    BUILTIN_CHANNEL_TYPES
        .iter()
        .find(|(name, _)| *name == builtin_name)
        .map(|(_, channel_type)| *channel_type)
}

/// Every `registerBuiltinType` call in `scripting/src/language_rules.cpp` that
/// supplies a channel type, in registration order.
pub const BUILTIN_CHANNEL_TYPES: &[(&str, &str)] = &[
    ("__engine_sim__float", "FloatNodeType"),
    ("__engine_sim__int", "IntNodeType"),
    ("__engine_sim__bool", "BoolNodeType"),
    ("__engine_sim__string", "StringNodeType"),
    ("__engine_sim__engine_channel", "EngineChannel"),
    ("__engine_sim__crankshaft_channel", "CrankshaftChannel"),
    ("__engine_sim__rod_journal_channel", "RodJournalChannel"),
    ("__engine_sim__connecting_rod_channel", "ConnectingRodChannel"),
    ("__engine_sim__piston_channel", "PistonChannel"),
    ("__engine_sim__cylinder_bank_channel", "CylinderBankChannel"),
    ("__engine_sim__function_channel", "FunctionChannel"),
    ("__engine_sim__cylinder_head_channel", "CylinderHeadChannel"),
    ("__engine_sim__camshaft_channel", "CamshaftChannel"),
    ("__engine_sim__intake_channel", "IntakeChannel"),
    ("__engine_sim__exhaust_system_channel", "ExhaustSystemChannel"),
    ("__engine_sim__ignition_module_channel", "IgnitionModuleChannel"),
    ("__engine_sim__ignition_wire_channel", "IgnitionWireChannel"),
    ("__engine_sim__fuel_channel", "FuelChannel"),
    ("__engine_sim__impulse_response_channel", "ImpulseResponseChannel"),
    ("__engine_sim__valvetrain_channel", "ValvetrainChannel"),
    ("__engine_sim__vehicle_channel", "VehicleChannel"),
    ("__engine_sim__transmission_channel", "TransmissionChannel"),
    ("__engine_sim__throttle_channel", "ThrottleChannel"),
    ("__engine_sim__literal_float", "FloatNodeType"),
    ("__engine_sim__literal_string", "StringNodeType"),
    ("__engine_sim__literal_int", "IntNodeType"),
    ("__engine_sim__literal_bool", "BoolNodeType"),
];

/// Every name `registerBuiltinType` registers, whether or not it carries a
/// channel type -- this is what `LanguageRules::checkBuiltinType` tests, and
/// therefore what decides `UndefinedBuiltinType` (`V0070`).
///
/// 81 calls register 80 distinct names (`__engine_sim__float_divide` is
/// registered twice, harmlessly: the rule map keeps the last one).
pub const BUILTIN_NODE_TYPES: &[&str] = &[
    "__engine_sim__float",
    "__engine_sim__int",
    "__engine_sim__bool",
    "__engine_sim__string",
    "__engine_sim__engine_channel",
    "__engine_sim__crankshaft_channel",
    "__engine_sim__rod_journal_channel",
    "__engine_sim__connecting_rod_channel",
    "__engine_sim__piston_channel",
    "__engine_sim__cylinder_bank_channel",
    "__engine_sim__function_channel",
    "__engine_sim__cylinder_head_channel",
    "__engine_sim__camshaft_channel",
    "__engine_sim__intake_channel",
    "__engine_sim__exhaust_system_channel",
    "__engine_sim__ignition_module_channel",
    "__engine_sim__ignition_wire_channel",
    "__engine_sim__fuel_channel",
    "__engine_sim__impulse_response_channel",
    "__engine_sim__valvetrain_channel",
    "__engine_sim__vehicle_channel",
    "__engine_sim__transmission_channel",
    "__engine_sim__throttle_channel",
    "__engine_sim__literal_float",
    "__engine_sim__literal_string",
    "__engine_sim__literal_int",
    "__engine_sim__literal_bool",
    "__engine_sim__int_to_float",
    "__engine_sim__int_to_string",
    "__engine_sim__string_to_int",
    "__engine_sim__float_negate",
    "__engine_sim__float_divide",
    "__engine_sim__float_multiply",
    "__engine_sim__float_add",
    "__engine_sim__float_subtract",
    "__engine_sim__int_multiply",
    "__engine_sim__int_add",
    "__engine_sim__int_subtract",
    "__engine_sim__int_divide",
    "__engine_sim__int_negate",
    "__engine_sim__set_engine",
    "__engine_sim__add_rod_journal",
    "__engine_sim__add_slave_journal",
    "__engine_sim__add_crankshaft",
    "__engine_sim__add_cylinder_bank",
    "__engine_sim__add_cylinder",
    "__engine_sim__add_sample",
    "__engine_sim__add_lobe",
    "__engine_sim__set_cylinder_head",
    "__engine_sim__connect_ignition_wire",
    "__engine_sim__add_ignition_module",
    "__engine_sim__k_28inH2O",
    "__engine_sim__k_carb",
    "__engine_sim__generate_harmonic_cam_lobe",
    "__engine_sim__set_application_settings",
    "__engine_sim__set_vehicle",
    "__engine_sim__set_transmission",
    "__engine_sim__add_gear",
    "__engine_sim__engine",
    "__engine_sim__rod_journal",
    "__engine_sim__crankshaft",
    "__engine_sim__connecting_rod",
    "__engine_sim__cylinder_bank",
    "__engine_sim__piston",
    "__engine_sim__function",
    "__engine_sim__cylinder_head",
    "__engine_sim__camshaft",
    "__engine_sim__exhaust_system",
    "__engine_sim__intake",
    "__engine_sim__ignition_module",
    "__engine_sim__ignition_wire",
    "__engine_sim__fuel",
    "__engine_sim__impulse_response",
    "__engine_sim__standard_valvetrain",
    "__engine_sim__vtec_valvetrain",
    "__engine_sim__vehicle",
    "__engine_sim__transmission",
    "__engine_sim__direct_throttle_linkage",
    "__engine_sim__governor",
    "__engine_sim__string_add",
];

/// `LanguageRules::checkBuiltinType`
pub fn is_builtin_node_type(name: &str) -> bool {
    BUILTIN_NODE_TYPES.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fundamental_channels_are_registered() {
        assert_eq!(builtin_channel_type("__engine_sim__float"), Some("FloatNodeType"));
        assert_eq!(builtin_channel_type("__engine_sim__literal_string"), Some("StringNodeType"));
        assert_eq!(
            builtin_channel_type("__engine_sim__exhaust_system_channel"),
            Some("ExhaustSystemChannel")
        );
    }

    #[test]
    fn builtins_without_a_channel_type_have_no_entry() {
        // Registered without a `ChannelType *`, so the original reports none.
        assert_eq!(builtin_channel_type("__engine_sim__int_to_float"), None);
        assert_eq!(builtin_channel_type("__engine_sim__nonsense"), None);
    }

    #[test]
    fn the_table_matches_the_registration_count() {
        assert_eq!(BUILTIN_CHANNEL_TYPES.len(), 27);
        assert_eq!(BUILTIN_NODE_TYPES.len(), 80);
    }

    #[test]
    fn every_channel_type_belongs_to_a_registered_builtin() {
        for (name, _) in BUILTIN_CHANNEL_TYPES {
            assert!(is_builtin_node_type(name), "{name} is not a registered builtin");
        }
    }

    #[test]
    fn builtin_lookup_matches_check_builtin_type() {
        assert!(is_builtin_node_type("__engine_sim__governor"));
        assert!(is_builtin_node_type("__engine_sim__float"));
        assert!(!is_builtin_node_type("__builtin"));
        assert!(!is_builtin_node_type(""));
    }
}
