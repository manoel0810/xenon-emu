use crate::loader::{XurScene, XusTable};
use std::collections::HashMap;

/// Runtime Manager for Microsoft's Xbox User Interface (XUI) subsystem
pub struct XuiManager {
    pub loaded_scenes: HashMap<String, XurScene>,
    pub string_table: Option<XusTable>,
    pub active_scene_name: Option<String>,
    pub registered_classes: Vec<String>,
}

impl XuiManager {
    pub fn new() -> Self {
        Self {
            loaded_scenes: HashMap::new(),
            string_table: None,
            active_scene_name: None,
            registered_classes: Vec::new(),
        }
    }

    /// Loads an authentic .xus localized string table
    pub fn set_string_table(&mut self, table: XusTable) {
        println!("[+] XUI: Registered Localized String Table with {} authentic strings", table.strings.len());
        self.string_table = Some(table);
    }

    /// Registers a parsed .xur visual scene into the XUI manager
    pub fn register_scene(&mut self, scene: XurScene) {
        println!("[+] XUI: Registered Scene '{}' (Root: {}, Children: {})",
            scene.name, scene.root.class_name, scene.root.children.len()
        );
        self.active_scene_name = Some(scene.name.clone());
        self.loaded_scenes.insert(scene.name.clone(), scene);
    }

    /// Finds a localized string or returns fallback
    pub fn get_string(&self, query: &str) -> Option<&String> {
        self.string_table.as_ref().and_then(|t| t.find(query))
    }

    /// Returns the currently active XUI scene
    pub fn get_active_scene(&self) -> Option<&XurScene> {
        self.active_scene_name
            .as_ref()
            .and_then(|name| self.loaded_scenes.get(name))
    }
}
