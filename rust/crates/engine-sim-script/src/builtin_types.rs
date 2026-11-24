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

use crate::ir::{BinOp, UnOp};
use crate::language_rules::LiteralKind;

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

/// The names `piranha::TestRules` registers (`piranha/test/test_rules.cpp`),
/// which is the rule set the original's own conformance suite compiles against.
/// 37 `registerBuiltinType` calls, 37 distinct names.
pub const PIRANHA_BUILTIN_NODE_TYPES: [&str; 37] = [
    "__piranha__int",
    "__piranha__float",
    "__piranha__string",
    "__piranha__vector",
    "__piranha__bool",
    "__piranha__vector_constructor",
    "__piranha__throw_runtime_error",
    "__piranha__int_probe",
    "__piranha__int_add",
    "__piranha__float_add",
    "__piranha__string_add",
    "__piranha__vector_add",
    "__piranha__int_subtract",
    "__piranha__float_subtract",
    "__piranha__vector_subtract",
    "__piranha__int_multiply",
    "__piranha__float_multiply",
    "__piranha__vector_multiply",
    "__piranha__int_divide",
    "__piranha__float_divide",
    "__piranha__vector_divide",
    "__piranha__int_negate",
    "__piranha__float_negate",
    "__piranha__vector_negate",
    "__piranha__int_positive",
    "__piranha__float_positive",
    "__piranha__vector_positive",
    "__piranha__bool_negate",
    "__piranha__literal_string",
    "__piranha__literal_int",
    "__piranha__literal_float",
    "__piranha__literal_bool",
    "__piranha__string_to_float",
    "__piranha__float_to_string",
    "__piranha__int_to_float",
    "__piranha__console_out",
    "__piranha__console_in",
];

/// `LanguageRules::checkBuiltinType` for the conformance rule set.
pub fn piranha_is_builtin_node_type(name: &str) -> bool {
    PIRANHA_BUILTIN_NODE_TYPES.contains(&name)
}

/// Which `LanguageRules` a compilation is using. The two differ only in the
/// builtin name table today; the operator tables will join them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rules {
    /// `es_script::LanguageRules` -- the engine's own scripts.
    #[default]
    EngineSim,
    /// `piranha::TestRules` -- the original's conformance fixtures.
    Piranha,
}

impl Rules {
    /// `IrNodeDefinition::validateBuiltinMappings`' first check.
    pub fn is_builtin_node_type(self, name: &str) -> bool {
        match self {
            Rules::EngineSim => is_builtin_node_type(name),
            Rules::Piranha => piranha_is_builtin_node_type(name),
        }
    }

    /// `LanguageRules::resolveChannelType` -- the channel type a builtin node
    /// was registered with. The engine's rules and the fixtures' rules register
    /// different *names* for the same `ChannelType` objects, so this has to be
    /// per ruleset.
    pub fn channel_type(self, builtin: &str) -> Option<&'static str> {
        let table = match self {
            Rules::EngineSim => crate::language_rules::ENGINE_SIM_BUILTIN_CHANNEL_TYPES,
            Rules::Piranha => crate::language_rules::PIRANHA_BUILTIN_CHANNEL_TYPES,
        };

        table
            .iter()
            .find(|rule| rule.builtin == builtin)
            .map(|rule| rule.channel_type)
    }

    /// `LanguageRules::resolveLiteralBuiltinType` -- which builtin node stands
    /// behind a literal of this type.
    pub fn literal_builtin(self, literal: LiteralKind) -> Option<&'static str> {
        let table = match self {
            Rules::EngineSim => crate::language_rules::ENGINE_SIM_LITERALS,
            Rules::Piranha => crate::language_rules::PIRANHA_LITERALS,
        };

        table.iter().find(|rule| rule.literal == literal).map(|rule| rule.builtin)
    }

    /// `LanguageRules::resolveOperatorBuiltinType`.
    ///
    /// A null operand type has no rule. Otherwise `OperatorMapping::operator==`
    /// is used, which is *reversible*: a rule whose operands are in the other
    /// order matches too, so `Add(Int, Float)` resolves through the registered
    /// `Add(Float, Int)` rule. `reversible` defaults to `true`
    /// (`include/language_rules.h:51`) and neither ruleset passes a fourth
    /// initializer, so every rule is reversible.
    pub fn operator_builtin(
        self,
        op: BinOp,
        left: Option<&str>,
        right: Option<&str>,
    ) -> Option<&'static str> {
        let (left, right) = (left?, right?);

        let table = match self {
            Rules::EngineSim => crate::language_rules::ENGINE_SIM_OPERATORS,
            Rules::Piranha => crate::language_rules::PIRANHA_OPERATORS,
        };

        table
            .iter()
            .find(|rule| {
                rule.op == op
                    && ((rule.left == left && rule.right == right)
                        || (rule.left == right && rule.right == left))
            })
            .map(|rule| rule.builtin)
    }

    /// `LanguageRules::resolveUnaryOperatorBuiltinType`.
    pub fn unary_builtin(self, op: UnOp, type_name: Option<&str>) -> Option<&'static str> {
        let type_name = type_name?;

        let table = match self {
            Rules::EngineSim => crate::language_rules::ENGINE_SIM_UNARY,
            Rules::Piranha => crate::language_rules::PIRANHA_UNARY,
        };

        table
            .iter()
            .find(|rule| rule.op == op && rule.type_name == type_name)
            .map(|rule| rule.builtin)
    }

    /// `LanguageRules::resolveConversionBuiltinType`.
    pub fn conversion_builtin(
        self,
        input: Option<&str>,
        output: Option<&str>,
    ) -> Option<&'static str> {
        let (input, output) = (input?, output?);

        let table = match self {
            Rules::EngineSim => crate::language_rules::ENGINE_SIM_CONVERSIONS,
            Rules::Piranha => crate::language_rules::PIRANHA_CONVERSIONS,
        };

        table
            .iter()
            .find(|rule| rule.input == input && rule.output == output)
            .map(|rule| rule.builtin)
    }

    /// `LanguageRules::checkConversion`
    pub fn check_conversion(self, input: Option<&str>, output: Option<&str>) -> bool {
        self.conversion_builtin(input, output).is_some()
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// The piranha table is a transcription, so it is checked against the file
    /// it came from: every `registerBuiltinType<...>("name")` call must appear
    /// in the table, and the table must not invent names. Angle brackets are
    /// balanced when scanning because `OperationNodeSpecialized<...>` nests.
    #[test]
    fn the_piranha_table_matches_test_rules_cpp() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../dependencies/submodules/piranha/test/test_rules.cpp");
        let source = std::fs::read_to_string(&path).expect("piranha submodule is checked out");

        let mut names = Vec::new();
        for (index, _) in source.match_indices("registerBuiltinType") {
            let mut rest = &source[index + "registerBuiltinType".len()..];
            rest = rest.trim_start();
            assert!(rest.starts_with('<'), "expected a template argument");

            let mut depth = 0usize;
            let mut end = 0usize;
            for (offset, character) in rest.char_indices() {
                match character {
                    '<' => depth += 1,
                    '>' => {
                        depth -= 1;
                        if depth == 0 {
                            end = offset + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }

            let after = &rest[end..];
            let start = after.find('"').expect("a name argument") + 1;
            let stop = after[start..].find('"').expect("a closing quote") + start;
            names.push(after[start..stop].to_string());
        }

        assert_eq!(names.len(), PIRANHA_BUILTIN_NODE_TYPES.len());
        for name in &names {
            assert!(
                piranha_is_builtin_node_type(name),
                "{} is registered by test_rules.cpp but missing from the table",
                name
            );
        }
    }

    /// `OperatorMapping::operator==` is reversible, so a rule registered with
    /// its operands in one order also matches the other order. `5 + 5.0` is an
    /// `Int + Float`, which only `Add(Float, Int)` covers.
    #[test]
    fn an_operator_rule_matches_with_its_operands_reversed() {
        let rules = Rules::Piranha;

        assert_eq!(
            rules.operator_builtin(BinOp::Add, Some("IntNodeType"), Some("FloatNodeType")),
            Some("__piranha__float_add")
        );
        assert_eq!(
            rules.operator_builtin(BinOp::Add, Some("FloatNodeType"), Some("IntNodeType")),
            Some("__piranha__float_add")
        );

        // Reversing does not conjure a rule out of nothing.
        assert_eq!(
            rules.operator_builtin(BinOp::Add, Some("StringNodeType"), Some("FloatNodeType")),
            None
        );
    }

    #[test]
    fn fundamental_channels_are_registered() {
        let rules = Rules::EngineSim;
        assert_eq!(rules.channel_type("__engine_sim__float"), Some("FloatNodeType"));
        assert_eq!(rules.channel_type("__engine_sim__literal_string"), Some("StringNodeType"));
        assert_eq!(
            rules.channel_type("__engine_sim__exhaust_system_channel"),
            Some("ExhaustSystemChannel")
        );
    }

    #[test]
    fn builtins_without_a_channel_type_have_no_entry() {
        // Registered without a `ChannelType *`, so the original reports none.
        let rules = Rules::EngineSim;
        assert_eq!(rules.channel_type("__engine_sim__int_to_float"), None);
        assert_eq!(rules.channel_type("__engine_sim__nonsense"), None);
    }

    #[test]
    fn the_table_matches_the_registration_count() {
        assert_eq!(crate::language_rules::ENGINE_SIM_BUILTIN_CHANNEL_TYPES.len(), 27);
        assert_eq!(BUILTIN_NODE_TYPES.len(), 80);
    }

    #[test]
    fn every_channel_type_belongs_to_a_registered_builtin() {
        for rule in crate::language_rules::ENGINE_SIM_BUILTIN_CHANNEL_TYPES {
            assert!(
                is_builtin_node_type(rule.builtin),
                "{} is not a registered builtin",
                rule.builtin
            );
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
