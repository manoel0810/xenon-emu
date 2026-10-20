use xenon_emu::gpu::GpuDisplay;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("[*] Generating preview snapshots of Xbox 360 boot animation...");
    let mut display = GpuDisplay::new("Xenon Boot Preview")?;

    // Render frame 70: Neon Green Aurora Ribbons
    display.boot_frame = 70;
    display.render_boot_animation();
    display.save_bmp("../bootanim_ribbons.bmp")?;
    println!("[+] Saved ../bootanim_ribbons.bmp (Frame 70: Neon Aurora Ribbons)");

    // Render frame 140: Sphere Coalescence & Ring of Light Chase
    display.boot_frame = 140;
    display.render_boot_animation();
    display.save_bmp("../bootanim_rol_chase.bmp")?;
    println!("[+] Saved ../bootanim_rol_chase.bmp (Frame 140: Sphere & RoL Chase)");

    // Render frame 180: Lens Flare Burst & "XBOX 360" Logo with Player 1 Locked
    display.boot_frame = 180;
    display.render_boot_animation();
    display.save_bmp("../bootanim_sphere_flare.bmp")?;
    println!("[+] Saved ../bootanim_sphere_flare.bmp (Frame 180: Lens Flare & Logo)");

    println!("[+] All boot animation preview frames rendered successfully!");
    Ok(())
}
