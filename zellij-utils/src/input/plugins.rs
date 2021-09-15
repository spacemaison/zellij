//! Plugins configuration metadata
use std::borrow::Borrow;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt::{self, Display};
use std::fs;
use std::path::{Path, PathBuf};

use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use url::Url;

use super::config::ConfigFromYaml;
use super::layout::{RunPlugin, RunPluginLocation};
use crate::setup;
pub use zellij_tile::data::PluginTag;

lazy_static! {
    static ref DEFAULT_CONFIG_PLUGINS: Plugins = {
        let cfg = String::from_utf8(setup::DEFAULT_CONFIG.to_vec()).unwrap();
        let cfg_yaml: ConfigFromYaml = serde_yaml::from_str(cfg.as_str()).unwrap();
        Plugins::try_from(cfg_yaml.plugins).unwrap()
    };
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct PluginsFromYaml(Vec<PluginFromYaml>);

/// Used in the config struct for plugin metadata
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Plugins(HashMap<PluginTag, Plugin>);

impl Plugins {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    /// Entrypoint from the config module
    pub fn get_plugins_with_default(user_plugins: Self) -> Self {
        let mut base_plugins = DEFAULT_CONFIG_PLUGINS.clone();
        base_plugins.0.extend(user_plugins.0);
        base_plugins
    }

    pub fn get(&self, run: impl Borrow<RunPlugin>) -> Option<Plugin> {
        let run = run.borrow();
        match &run.location {
            // FIXME
            RunPluginLocation::File(path) => Some(Plugin {
                path: path.clone(),
                tag: PluginTag::default(),
                run: PluginType::OncePerPane(None),
                _allow_exec_host_cmd: run._allow_exec_host_cmd,
                options: serde_json::to_string(&run.options).unwrap(),
            }),
            RunPluginLocation::Zellij(tag) => match self.0.get(tag).cloned() {
                Some(mut plugin) => {
                    let mut options: serde_yaml::Value =
                        serde_yaml::from_str(&plugin.options).unwrap();

                    if let Some(run_options) = run.options.clone() {
                        merge_yaml(&mut options, run_options);
                    }

                    plugin.options = serde_json::to_string(&options).unwrap();

                    Some(plugin)
                }
                None => None,
            },
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Plugin> {
        self.0.values()
    }
}

impl Default for Plugins {
    fn default() -> Self {
        Self::get_plugins_with_default(Plugins::new())
    }
}

impl TryFrom<PluginsFromYaml> for Plugins {
    type Error = PluginsError;

    fn try_from(yaml: PluginsFromYaml) -> Result<Self, PluginsError> {
        let mut plugins = HashMap::new();
        for plugin in yaml.0 {
            if plugins.contains_key(&plugin.tag) {
                return Err(PluginsError::DuplicatePlugins(plugin.tag));
            }
            plugins.insert(plugin.tag.clone(), plugin.into());
        }

        Ok(Plugins(plugins))
    }
}

impl From<PluginFromYaml> for Plugin {
    fn from(plugin: PluginFromYaml) -> Self {
        Plugin {
            path: plugin.path,
            tag: plugin.tag,
            run: match plugin.run {
                PluginTypeFromYaml::OncePerPane => PluginType::OncePerPane(None),
                PluginTypeFromYaml::Headless => PluginType::Headless,
            },
            _allow_exec_host_cmd: plugin._allow_exec_host_cmd,
            options: plugin
                .options
                .map(|options| serde_json::to_string(&options).unwrap())
                .unwrap_or_else(|| "null".into()),
        }
    }
}

/// Plugin metadata
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Plugin {
    /// Path of the plugin, see resolve_wasm_bytes for resolution semantics
    pub path: PathBuf,
    /// Tag used to identify the plugin in layout and config yaml files
    pub tag: PluginTag,
    /// Plugin type
    pub run: PluginType,
    /// Allow command execution from plugin
    pub _allow_exec_host_cmd: bool,
    /// Encoded JSON options string. This is a JSON string rather than a serde_json::Value because
    /// the bincode crate that used to serialize/deserialize messages between the client and server
    /// doesn't support unstructured data formats.
    pub options: String,
}

impl Plugin {
    /// Resolve wasm plugin bytes for the plugin path and given plugin directory. Attempts to first
    /// resolve the plugin path as an absolute path, then adds a ".wasm" extension to the path and
    /// resolves that, finally we use the plugin directoy joined with the path with an appended
    /// ".wasm" extension. So if our path is "tab-bar" and the given plugin dir is
    /// "/home/bob/.zellij/plugins" the lookup chain will be this:
    ///
    /// ```bash
    ///   /tab-bar
    ///   /tab-bar.wasm
    ///   /home/bob/.zellij/plugins/tab-bar.wasm
    /// ```
    ///
    pub fn resolve_wasm_bytes(&self, plugin_dir: &Path) -> Option<Vec<u8>> {
        fs::read(&self.path)
            .or_else(|_| fs::read(&self.path.with_extension("wasm")))
            .or_else(|_| fs::read(plugin_dir.join(&self.path).with_extension("wasm")))
            .ok()
    }

    /// Sets the tab index inside of the plugin type of the run field.
    pub fn set_tab_index(&mut self, tab_index: usize) {
        match self.run {
            PluginType::OncePerPane(..) => {
                self.run = PluginType::OncePerPane(Some(tab_index));
            }
            PluginType::Headless => {}
        }
    }
}

/// Type of the plugin. Defaults to OncePerPane.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginType {
    // TODO: A plugin with output thats cloned across every pane in a tab, or across the entire
    // application might be useful
    // OncePerTab
    // Static
    /// Starts immediately when Zellij is started and runs without a visible pane
    Headless,
    /// Runs when declared inside a layout file
    OncePerPane(Option<usize>), // tab_index
}

impl Default for PluginType {
    fn default() -> Self {
        Self::OncePerPane(None)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct PluginFromYaml {
    pub path: PathBuf,
    pub tag: PluginTag,
    #[serde(default)]
    pub run: PluginTypeFromYaml,
    #[serde(default)]
    pub _allow_exec_host_cmd: bool,
    #[serde(default)]
    pub options: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginTypeFromYaml {
    Headless,
    OncePerPane,
}

impl Default for PluginTypeFromYaml {
    fn default() -> Self {
        Self::OncePerPane
    }
}

#[derive(Debug, PartialEq)]
pub enum PluginsError {
    DuplicatePlugins(PluginTag),
    InvalidUrl(Url),
    InvalidPluginLocation(PathBuf),
}

impl std::error::Error for PluginsError {}
impl Display for PluginsError {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        match self {
            PluginsError::DuplicatePlugins(tag) => write!(
                formatter,
                "Duplication in plugin tag names is not allowed: '{}'",
                String::from(tag.clone())
            ),
            PluginsError::InvalidUrl(url) => write!(
                formatter,
                "Only 'file:' and 'zellij:' url schemes are supported for plugin lookup. '{}' does not match either.",
                url
            ),
            PluginsError::InvalidPluginLocation(path) => write!(
                formatter,
                "Could not find plugin at the path: '{:?}'", path
            ),
        }
    }
}

fn merge_yaml(a: &mut serde_yaml::Value, b: serde_yaml::Value) {
    match (a, b) {
        (a @ &mut serde_yaml::Value::Mapping(_), serde_yaml::Value::Mapping(b)) => {
            let a = a.as_mapping_mut().unwrap();
            for (k, v) in b {
                if v.is_sequence() && a.contains_key(&k) && a[&k].is_sequence() {
                    let mut _b = a.get(&k).unwrap().as_sequence().unwrap().to_owned();
                    _b.append(&mut v.as_sequence().unwrap().to_owned());
                    a[&k] = serde_yaml::Value::from(_b);
                    continue;
                }
                if !a.contains_key(&k) {
                    a.insert(k.to_owned(), v.to_owned());
                } else {
                    merge_yaml(&mut a[&k], v);
                }
            }
        }
        (a, b) => *a = b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::config::ConfigError;
    use std::convert::TryInto;

    #[test]
    fn try_from_yaml_fails_when_duplicate_tag_names_are_present() -> Result<(), ConfigError> {
        let ConfigFromYaml { plugins, .. } = serde_yaml::from_str(
            "
            plugins:
                - path: /foo/bar/baz.wasm
                  tag: boo
                - path: /foo/bar/boo.wasm
                  tag: boo
        ",
        )?;

        assert_eq!(
            Plugins::try_from(plugins),
            Err(PluginsError::DuplicatePlugins(PluginTag::new("boo")))
        );

        Ok(())
    }

    #[test]
    fn plugins_allow_options() -> Result<(), ConfigError> {
        let ConfigFromYaml { plugins, .. } = serde_yaml::from_str(
            "
            plugins:
                - path: /foo/bar/baz.wasm
                  tag: boo
                  options:
                    foo: bar
                    you: too
        ",
        )?;

        let plugins: Plugins = plugins.try_into()?;

        assert_eq!(
            plugins.get(RunPlugin {
                location: RunPluginLocation::Zellij(PluginTag::new("boo")),
                options: None,
                ..Default::default()
            }),
            Some(Plugin {
                _allow_exec_host_cmd: false,
                path: PathBuf::from("/foo/bar/baz.wasm"),
                tag: PluginTag::new("boo"),
                run: PluginType::OncePerPane(None),
                options: {
                    let yaml: serde_yaml::Value = serde_yaml::from_str("foo: bar\nyou: too")?;
                    serde_json::to_string(&yaml).unwrap()
                }
            })
        );
        Ok(())
    }

    #[test]
    fn plugins_allow_overriding_options_from_run_configuration() -> Result<(), ConfigError> {
        let ConfigFromYaml { plugins, .. } = serde_yaml::from_str(
            "
            plugins:
                - path: /foo/bar/baz.wasm
                  tag: boo
                  options:
                    foo: bar
                    you: too
        ",
        )?;

        let plugins: Plugins = plugins.try_into()?;

        assert_eq!(
            plugins.get(RunPlugin {
                location: RunPluginLocation::Zellij(PluginTag::new("boo")),
                options: serde_yaml::from_str("foo: boo").unwrap(),
                ..Default::default()
            }),
            Some(Plugin {
                _allow_exec_host_cmd: false,
                path: PathBuf::from("/foo/bar/baz.wasm"),
                tag: PluginTag::new("boo"),
                run: PluginType::OncePerPane(None),
                options: {
                    let yaml: serde_yaml::Value = serde_yaml::from_str(
                        "
                           foo: boo
                           you: too
                   ",
                    )
                    .unwrap();
                    serde_json::to_string(&yaml).unwrap()
                }
            })
        );
        Ok(())
    }

    #[test]
    fn plugins_allow_options_entirely_from_run_configuration() -> Result<(), ConfigError> {
        let ConfigFromYaml { plugins, .. } = serde_yaml::from_str(
            "
            plugins:
                - path: /foo/bar/baz.wasm
                  tag: boo
        ",
        )?;

        let plugins: Plugins = plugins.try_into()?;

        assert_eq!(
            plugins.get(RunPlugin {
                location: RunPluginLocation::Zellij(PluginTag::new("boo")),
                options: serde_yaml::from_str("foo: boo").unwrap(),
                ..Default::default()
            }),
            Some(Plugin {
                _allow_exec_host_cmd: false,
                path: PathBuf::from("/foo/bar/baz.wasm"),
                tag: PluginTag::new("boo"),
                run: PluginType::OncePerPane(None),
                options: {
                    let yaml: serde_yaml::Value = serde_yaml::from_str("foo: boo").unwrap();
                    serde_json::to_string(&yaml).unwrap()
                }
            })
        );
        Ok(())
    }

    #[test]
    fn plugins_can_inject_defaults() -> Result<(), ConfigError> {
        let ConfigFromYaml { plugins, .. } = serde_yaml::from_str(
            "
            plugins:
                - path: boo.wasm
                  tag: boo
        ",
        )?;
        let plugins = Plugins::get_plugins_with_default(plugins.try_into()?);

        assert_eq!(plugins.iter().collect::<Vec<_>>().len(), 4);
        Ok(())
    }

    #[test]
    fn plugins_with_defaults_allow_overriding() -> Result<(), ConfigError> {
        let ConfigFromYaml { plugins, .. } = serde_yaml::from_str(
            "
            plugins:
                - path: boo.wasm
                  tag: tab-bar
        ",
        )?;
        let plugins = Plugins::get_plugins_with_default(plugins.try_into()?);

        assert_eq!(
            plugins.get(RunPlugin {
                location: RunPluginLocation::Zellij(PluginTag::new("tab-bar")),
                ..Default::default()
            }),
            Some(Plugin {
                _allow_exec_host_cmd: false,
                path: PathBuf::from("boo.wasm"),
                tag: PluginTag::new("tab-bar"),
                run: PluginType::OncePerPane(None),
                options: "null".into()
            })
        );

        Ok(())
    }
}
