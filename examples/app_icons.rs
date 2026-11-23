//! Export platform icon resources from the shared Terminal Note renderer.
use anyhow::Result;
use std::{path::Path, process::Command};
use textamp::{
    app::theme::ThemeName,
    icon::{self, Appearance, Platform},
};

fn main() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let alternate_icons = ThemeName::all()
        .iter()
        .filter(|theme| **theme != ThemeName::System)
        .map(|theme| format!("AppIcon-{}", theme.config_name()))
        .collect::<Vec<_>>()
        .join(" ");
    std::fs::write(
        root.join("ios/IconAssets.xcconfig"),
        format!("ASSETCATALOG_COMPILER_ALTERNATE_APPICON_NAMES = {alternate_icons}\n"),
    )?;
    let ios = root.join("ios/Textamp/Assets.xcassets");
    let mac = root.join("packaging/macos-desktop/assets");
    let variants = root.join("assets/icons");
    std::fs::create_dir_all(&variants)?;
    for &theme in ThemeName::all() {
        let name = if theme == ThemeName::System {
            "AppIcon".into()
        } else {
            format!("AppIcon-{}", theme.config_name())
        };
        let directory = ios.join(format!("{name}.appiconset"));
        std::fs::create_dir_all(&directory)?;
        let ios_image = image::DynamicImage::ImageRgba8(icon::render(
            theme,
            Appearance::Light,
            1024,
            Platform::Ios,
        ))
        .into_rgb8();
        ios_image.save(directory.join("Icon.png"))?;
        icon::render(theme, Appearance::Light, 1024, Platform::MacOs)
            .save(variants.join(format!("{}.png", theme.config_name())))?;
        let mut images = if theme == ThemeName::System {
            vec![
                serde_json::json!({"filename":"Icon.png","idiom":"universal","platform":"ios","size":"1024x1024"}),
            ]
        } else {
            let mut images = vec![
                serde_json::json!({"filename":"Icon.png","idiom":"ios-marketing","scale":"1x","size":"1024x1024"}),
            ];
            // Device slots supply the native sizes for iPhone and iPad icons.
            for &(idiom, size, scale, pixels) in &[
                ("iphone", "20x20", 2, 40),
                ("iphone", "20x20", 3, 60),
                ("iphone", "29x29", 2, 58),
                ("iphone", "29x29", 3, 87),
                ("iphone", "40x40", 2, 80),
                ("iphone", "40x40", 3, 120),
                ("iphone", "60x60", 2, 120),
                ("iphone", "60x60", 3, 180),
                ("ipad", "20x20", 1, 20),
                ("ipad", "20x20", 2, 40),
                ("ipad", "29x29", 1, 29),
                ("ipad", "29x29", 2, 58),
                ("ipad", "40x40", 1, 40),
                ("ipad", "40x40", 2, 80),
                ("ipad", "76x76", 1, 76),
                ("ipad", "76x76", 2, 152),
                ("ipad", "83.5x83.5", 2, 167),
            ] {
                let filename = format!("Icon-{idiom}-{size}@{scale}x.png");
                image::imageops::resize(
                    &ios_image,
                    pixels,
                    pixels,
                    image::imageops::FilterType::Lanczos3,
                )
                .save(directory.join(&filename))?;
                images.push(serde_json::json!({"filename":filename,"idiom":idiom,"size":size,"scale":format!("{scale}x")}));
            }
            images
        };
        if theme == ThemeName::System {
            image::DynamicImage::ImageRgba8(icon::render(
                theme,
                Appearance::Dark,
                1024,
                Platform::Ios,
            ))
            .into_rgb8()
            .save(directory.join("Dark.png"))?;
            icon::render(theme, Appearance::Dark, 1024, Platform::MacOs)
                .save(variants.join("system-dark.png"))?;
            images.push(serde_json::json!({"filename":"Dark.png","idiom":"universal","platform":"ios","size":"1024x1024","appearances":[{"appearance":"luminosity","value":"dark"}]}));
        }
        std::fs::write(
            directory.join("Contents.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"images":images,"info":{"author":"Textamp","version":1}}),
            )?,
        )?;
    }
    let iconset = mac.join("Textamp.iconset");
    std::fs::create_dir_all(&iconset)?;
    let master = icon::render(ThemeName::System, Appearance::Light, 1024, Platform::MacOs);
    for size in [16, 32, 128, 256, 512] {
        for scale in [1, 2] {
            let suffix = if scale == 2 { "@2x" } else { "" };
            image::imageops::resize(
                &master,
                size * scale,
                size * scale,
                image::imageops::FilterType::Lanczos3,
            )
            .save(iconset.join(format!("icon_{size}x{size}{suffix}.png")))?;
        }
    }
    let status = Command::new("iconutil")
        .arg("-c")
        .arg("icns")
        .arg(&iconset)
        .arg("-o")
        .arg(mac.join("Textamp.icns"))
        .status()?;
    anyhow::ensure!(status.success(), "iconutil failed");
    std::fs::remove_dir_all(iconset)?;
    Ok(())
}
