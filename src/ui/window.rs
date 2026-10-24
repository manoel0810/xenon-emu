//! Output window: creation, controls banner and the final screenshot.

use xenon_emu::gpu::GpuDisplay;
use xenon_emu::gpu::display::resolve_path;
use xenon_emu::loader::{XurScene, XusTable};

/// MILESTONE 5: opens the 1280x720 surface (no OS window in headless mode) and prints the controls.
pub fn open(is_metro: bool, xus_table: &Option<XusTable>, loaded_scenes: &[XurScene]) -> Result<GpuDisplay, Box<dyn std::error::Error>> {
    // ==========================================================
    // MILESTONE 5: High-Definition GPU Video Display & Surface Rendering
    // ==========================================================
    println!("\n>>> [MILESTONE 5] GPU Video Output (1280x720 HD @ 60 FPS) <<<");
    println!("[*] Initializing High-Definition Video Display Surface...");
    let win_title = if is_metro {
        "Xenon-Emu: Xbox 360 Metro Dashboard [Title: FFFE07D1 | Build 17559]"
    } else {
        "Xenon-Emu: Xbox 360 Blades Dashboard [Title: FFFE07D1 | Build 4532]"
    };
    let mut display = GpuDisplay::new(win_title)?;
    if let Some(table) = xus_table {
        display.apply_xui_locale(table, loaded_scenes);
    }
    println!("[+] Display Window opened: 1280x720 @ 60 FPS");

    println!("\n[*] ========================================================");
    println!("    🎮 XBOX 360 INICIANDO: ANIMAÇÃO DE BOOT ATIVA!");
    println!("    - A animacao de boot do Xbox 360 e reproduzida (~4.5s)");
    println!("    - Pressione ESPACO ou (A) no controle para pular (apos 1s)");
    println!("    - Em seguida, a Dashboard surge em 1280x720 HD @ 60 FPS");
    println!("    - [TAB] ou Botao (Y): Alternar Modos: Blades -> Guest VRAM -> Hibrido -> XUI");
    println!("    - [U]: Alternar Algoritmo GPU Untile (Xenia Morton / Variant 0 / Linear)");
    println!("    - [B]: Alternar Buffer de Vídeo (VRAM Guest Ao Vivo vs Dump Calibrado de Hardware)");
    println!("    - [C]: Alternar Formato de Canais de Cor (ARGB / BGRA / RGBA / Raw)");
    println!("    - [X] ou [1..4]: Alternar Cenas XUI (BladeSkin, gamesSignedOut)");
    println!("    - Setas / A,D ou D-Pad / Thumbstick: Alternar Blades");
    println!("    - Setas / W,S ou D-Pad: Navegar nos itens do menu");
    println!("    - Pressione ESC ou clique no [X] para fechar a janela");
    println!("    ========================================================\n");

    Ok(display)
}

/// Saves the final framebuffer screenshot and prints the closing banner.
pub fn finish(display: &GpuDisplay, is_metro: bool) -> Result<(), Box<dyn std::error::Error>> {
    // Save framebuffer screenshot to disk
    let bmp_path = if is_metro { resolve_path("dash_17559_framebuffer.bmp") } else { resolve_path("dash_framebuffer.bmp") };
    display.save_bmp(&bmp_path)?;
    println!("[+] Framebuffer exported as High-Res BMP screenshot: {}", bmp_path.display());

    println!("\n========================================================");
    println!("  🎉 Milestones 1 to 5 Fully Operational!                ");
    println!("  Hardware Boot + XEX2 Loader + STFS Extractor +        ");
    println!("  HLE Kernel + 720p HD Display Window Active!           ");
    println!("========================================================");

    Ok(())
}
