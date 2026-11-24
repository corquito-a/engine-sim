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
    }
}
