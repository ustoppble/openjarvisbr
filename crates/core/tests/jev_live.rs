//! Uma chamada real ao Jev. Roda só com TYPESAFE_API_KEY:
//! `TYPESAFE_API_KEY=... cargo test -p openjarvisbr-core --test jev_live -- --ignored --nocapture`
use openjarvisbr_core::reflex::judge::{JevClient, Judge};
use openjarvisbr_core::reflex::questions::{Question, Questions};

#[tokio::test]
#[ignore]
async fn jev_responde_em_menos_de_um_segundo() {
    let key = std::env::var("TYPESAFE_API_KEY").expect("TYPESAFE_API_KEY");
    let client = JevClient::new(key, "jev-latest".into()).unwrap();
    let mut q = Questions::default();
    q.insert(
        "intent",
        Question::choice(
            "O que o usuário pede em state?",
            [
                ("open_app", "Abrir um aplicativo instalado"),
                ("open_site", "Abrir um site"),
                ("none", "Nenhuma ação"),
            ],
        ),
    );
    q.insert(
        "app",
        Question::choice(
            "Qual aplicativo, se for abrir um?",
            [
                ("Safari", "navegador Safari"),
                ("Spotify", "player Spotify"),
                ("none", "nenhum destes"),
            ],
        ),
    );
    let t0 = std::time::Instant::now();
    let a = client
        .ask("abre o safari pra mim", &q)
        .await
        .expect("resposta");
    let ms = t0.elapsed().as_millis();
    eprintln!("latência {ms} ms · usage {:?}", a.usage);
    assert_eq!(a.choice("intent").map(|c| c.0), Some("open_app"));
    assert_eq!(a.choice("app").map(|c| c.0), Some("Safari"));
    assert!(ms < 1000, "latência {ms} ms");
}
