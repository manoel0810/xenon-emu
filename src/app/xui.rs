//! ETAPA 1: XUI resources from disk (.xus string table and .xur scenes).

use xenon_emu::gpu::display::resolve_path;
use xenon_emu::kernel::XuiManager;
use xenon_emu::loader::{XurScene, XusTable};

pub struct XuiAssets {
    pub xus_table: Option<XusTable>,
    pub loaded_scenes: Vec<XurScene>,
}

pub fn load() -> XuiAssets {
    // ==========================================================
    // ETAPA 1: Authentic Microsoft XUI Resource Engine (.xur & .xus)
    // ==========================================================
    println!("\n>>> [ETAPA 1] Genuine Microsoft XUI Resource Engine (.XUR & .XUS) <<<");
    let mut xui_mgr = XuiManager::new();

    // 1. Load Portuguese (pt-br) localized string table from dump
    let pt_strings_path = resolve_path("extracted_xui/pt-br/Strings.xus");
    let xus_table = if pt_strings_path.exists() {
        println!("[*] Loading genuine Microsoft Portuguese string table (.xus)...");
        match XusTable::load_file(&pt_strings_path) {
            Ok(table) => {
                println!("[+] Successfully loaded {} Portuguese strings from dump!", table.strings.len());
                for (idx, s) in table.strings.iter().take(5).enumerate() {
                    println!("    [{:02}] \"{}\"", idx + 1, s);
                }
                Some(table)
            }
            Err(e) => {
                println!("[!] Failed to load {}: {}", pt_strings_path.display(), e);
                None
            }
        }
    } else {
        None
    };

    if let Some(ref t) = xus_table {
        xui_mgr.set_string_table(t.clone());
    }

    // 2. Load authentic Microsoft XUI Visual Scenes (.xur)
    let mut loaded_scenes = Vec::new();
    let xur_files = [
        ("dashmain", resolve_path("extracted_xui/dashmain.xur")),
        ("BladeSkin", resolve_path("extracted_xui/BladeSkin.xur")),
        ("gamesSignedOut", resolve_path("extracted_xui/gamesSignedOut.xur")),
        ("mediaSignedOut", resolve_path("extracted_xui/mediaSignedOut.xur")),
        ("liveSignedOutUI", resolve_path("extracted_xui/liveSignedOutUI.xur")),
    ];

    for (_sname, spath) in &xur_files {
        if spath.exists() {
            match XurScene::load_file(spath) {
                Ok(scene) => {
                    println!("[+] Parsed authentic XUI Scene '{}': Root Class '{}', {} Child Nodes, {} Classes",
                        scene.name, scene.root.class_name, scene.root.children.len(), scene.classes.len()
                    );
                    for (c_idx, child) in scene.root.children.iter().take(4).enumerate() {
                        println!("    -> Node #{:02}: id='{}', class='{}', bounds=({:.0}, {:.0}, {:.0}, {:.0})",
                            c_idx + 1, child.id, child.class_name, child.x, child.y, child.width, child.height
                        );
                    }
                    xui_mgr.register_scene(scene.clone());
                    loaded_scenes.push(scene);
                }
                Err(e) => println!("  [!] XUR parse error for {}: {}", spath.display(), e),
            }
        }
    }

    XuiAssets { xus_table, loaded_scenes }
}
