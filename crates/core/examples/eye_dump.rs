//! Imprime o inventário do Olho: `cargo run -p openjarvisbr-core --example eye_dump`
use openjarvisbr_core::reflex::eye::Eye;

#[tokio::main]
async fn main() {
    // TEMP até card A: trocar por `config::load_reflex().sites` quando
    // `[[reflex.sites]]` existir no config.
    let sites = Vec::new();
    let eye = Eye::start(sites);
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let inv = eye.snapshot();
    println!("instalados: {}", inv.installed_apps.len());
    println!("rodando ({}):", inv.running_apps.len());
    for app in &inv.running_apps {
        println!("  • {}", app.name);
    }
    println!("sites ({}):", inv.sites.len());
    for site in &inv.sites {
        println!("  • {} → {}", site.name, site.url);
    }
}
