//! Optional `quasar-config` value bridge. Games own persistence and validation.
use crate::UiValue;
use quasar_config::ConfigValue;

pub fn config_to_value(value: &ConfigValue) -> UiValue {
    match value {
        ConfigValue::Bool(value) => UiValue::Bool(*value),
        ConfigValue::Integer(value) => UiValue::Float(*value as f32),
        ConfigValue::Unsigned(value) => UiValue::UInt((*value).min(u64::from(u32::MAX)) as u32),
        ConfigValue::Float(value) => UiValue::Float(*value as f32),
        ConfigValue::String(value) | ConfigValue::Json(value) => UiValue::Text(value.clone()),
    }
}

pub fn value_to_config(value: &UiValue) -> ConfigValue {
    match value {
        UiValue::Float(value) => ConfigValue::Float(f64::from(*value)),
        UiValue::Bool(value) => ConfigValue::Bool(*value),
        UiValue::UInt(value) => ConfigValue::Unsigned(u64::from(*value)),
        UiValue::Text(value) => ConfigValue::String(value.clone()),
    }
}
