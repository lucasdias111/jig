use gpui_kit::TestAppContext;
use jig_ai::template_named;

use super::*;

#[gpui_kit::test]
fn connecting_with_a_key_makes_the_provider_usable(cx: &mut TestAppContext) {
    let dir = init_temp(cx);
    let openai = template_named("openai").unwrap();
    cx.update(|cx| {
        if std::env::var("OPENAI_API_KEY").is_err() {
            connect(openai, None, cx).unwrap();
            let provider = config(cx).unwrap().provider("openai").cloned().unwrap();
            assert!(!is_ready(&provider, cx), "no key yet");
        }
        connect(openai, Some("sk-test"), cx).unwrap();
        let provider = config(cx).unwrap().provider("openai").cloned().unwrap();
        assert_eq!(
            api_key(&provider, cx),
            Some(("sk-test".into(), KeySource::File))
        );
        assert_eq!(models("openai", cx), Some(ModelList::Loading));

        set_agent("openai/gpt-test", cx).unwrap();
        assert_eq!(
            agent_target_for(None, cx).unwrap().model,
            "jig-openai/gpt-test"
        );
    });
    let file = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    assert!(file.contains("agent = \"openai/gpt-test\""));

    cx.update(|cx| {
        disconnect("openai", cx).unwrap();
        assert!(
            agent_target_for(None, cx).is_err(),
            "the agent needs a new model"
        );
        assert_eq!(models("openai", cx), None);
    });
    let keys = std::fs::read_to_string(dir.path().join("keys.toml")).unwrap();
    assert!(!keys.contains("sk-test"), "disconnecting forgets the key");
}

#[gpui_kit::test]
fn the_first_provider_starts_the_agent_on_a_fast_model(cx: &mut TestAppContext) {
    let _dir = init_temp(cx);
    cx.update(|cx| {
        assert!(
            config(cx).unwrap().providers.is_empty(),
            "none to begin with"
        );
        connect(template_named("anthropic").unwrap(), None, cx).unwrap();
        let listed =
            |names: &[&str]| ModelList::Loaded(names.iter().map(|name| name.to_string()).collect());
        cx.update_global::<Providers, _>(|providers, _| {
            providers.loads.clear();
            providers.models.insert(
                "anthropic".into(),
                listed(&["claude-opus-5-5", "claude-haiku-5-5"]),
            );
        });
        start_on_a_model("anthropic", cx);
        assert_eq!(
            config(cx).unwrap().agent.as_deref(),
            Some("anthropic/claude-haiku-5-5")
        );
        // A model already picked stays.
        set_agent("anthropic/claude-opus-5-5", cx).unwrap();
        start_on_a_model("anthropic", cx);
        assert_eq!(
            config(cx).unwrap().agent.as_deref(),
            Some("anthropic/claude-opus-5-5")
        );
    });
}

#[gpui_kit::test]
fn the_agent_gets_its_provider_handed_to_opencode(cx: &mut TestAppContext) {
    let _dir = init_temp(cx);
    cx.update(|cx| {
        connect(template_named("ollama").unwrap(), None, cx).unwrap();
        set_agent("ollama/qwen3:8b", cx).unwrap();
        let target = agent_target_for(None, cx).unwrap();
        assert_eq!(target.model, "jig-ollama/qwen3:8b");
        assert!(target.config.unwrap()["provider"]["jig-ollama"].is_object());

        connect(template_named("opencode-go").unwrap(), None, cx).unwrap();
        set_agent("opencode-go/glm-5.3", cx).unwrap();
        if std::env::var("OPENCODE_API_KEY").is_err() {
            assert!(
                agent_target_for(None, cx)
                    .unwrap_err()
                    .contains("has no API key")
            );
        }
    });
}
