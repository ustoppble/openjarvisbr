use std::process::Command;

#[test]
fn fala_sem_candidato_explica_que_jev_nao_foi_consultado() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("relógio válido")
        .as_nanos();
    let home = std::env::temp_dir().join(format!("jarvis-reflex-cli-{unique}"));
    std::fs::create_dir_all(&home).expect("cria HOME temporário");
    let output = Command::new(env!("CARGO_BIN_EXE_jarvis"))
        .args(["reflex", "que horas são?"])
        .env("HOME", &home)
        .output()
        .expect("executa jarvis reflex");
    let _ = std::fs::remove_dir_all(home);

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout utf-8");
    assert!(stdout.contains("decisão: Nothing"), "{stdout}");
    assert!(stdout.contains("Jev não consultado"), "{stdout}");
}
