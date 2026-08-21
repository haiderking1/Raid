use std::sync::Arc;

use tokio::runtime::Handle;
use tokio::task::JoinHandle;

use crate::backend::opencode::ResolvedModel;
use crate::config::{
    connected_provider, filter_model_indices, load_connected_catalog_from_disk,
    refresh_connected_catalog, refresh_connected_catalog_async, save_default_model,
    save_text_generation_model, RaidSettings,
};
use crate::frontend::composer::{ComposerAction, ComposerState};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelAction {
    None,
    Cancelled,
    Selected { target: ModelTarget },
    Error { message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelTarget {
    Chat,
    TextGeneration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoadState {
    Ready,
    Loading,
    Error,
}

#[derive(Debug)]
pub struct ModelFlow {
    active: bool,
    target: ModelTarget,
    models: Arc<Vec<ResolvedModel>>,
    filtered: Vec<usize>,
    selected: usize,
    filter_query: String,
    current_model_id: String,
    status: String,
    load_state: LoadState,
    refresh: Option<JoinHandle<Result<crate::backend::opencode::OpenCodeCatalog, String>>>,
}

impl Default for ModelFlow {
    fn default() -> Self {
        Self {
            active: false,
            target: ModelTarget::Chat,
            models: Arc::new(Vec::new()),
            filtered: Vec::new(),
            selected: 0,
            filter_query: String::new(),
            current_model_id: String::new(),
            status: String::new(),
            load_state: LoadState::Ready,
            refresh: None,
        }
    }
}

impl ModelFlow {
    pub fn active(&self) -> bool {
        self.active
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn models(&self) -> &[ResolvedModel] {
        &self.models
    }

    pub fn filtered(&self) -> &[usize] {
        &self.filtered
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn start(&mut self, runtime: &Handle, target: ModelTarget) {
        *self = Self::default();
        self.active = true;
        self.target = target;
        let settings = RaidSettings::load();
        self.current_model_id = match target {
            ModelTarget::Chat => settings.model_id(),
            ModelTarget::TextGeneration => settings.text_generation_model_id(),
        }
        .to_string();

        match connected_provider() {
            Ok(provider) => {
                let _ = provider;
                self.status.clear();
                match load_connected_catalog_from_disk() {
                    Ok((catalog, _)) => {
                        self.replace_models(catalog.models, None);
                        self.spawn_refresh(runtime);
                    }
                    Err(_) => {
                        self.load_state = LoadState::Loading;
                        self.status = "loading models…".into();
                        match refresh_connected_catalog(runtime) {
                            Ok(catalog) => {
                                self.replace_models(catalog.models, None);
                                self.load_state = LoadState::Ready;
                                self.status.clear();
                            }
                            Err(error) => {
                                self.load_state = LoadState::Error;
                                self.status = error.clone();
                            }
                        }
                    }
                }
            }
            Err(error) => {
                self.load_state = LoadState::Error;
                self.status = error;
            }
        }
    }

    pub fn poll(&mut self, runtime: &Handle) {
        let Some(handle) = self.refresh.take() else {
            return;
        };
        if handle.is_finished() {
            if let Ok(Ok(catalog)) = runtime.block_on(handle) {
                let preserve = self.selected_model_id();
                self.replace_models(catalog.models, preserve.as_deref());
                if self.load_state == LoadState::Loading {
                    self.load_state = LoadState::Ready;
                    self.status.clear();
                }
            }
            return;
        }
        self.refresh = Some(handle);
    }

    pub fn apply_filter(&mut self, query: &str) {
        self.filter_query = query.to_string();
        self.filtered = filter_model_indices(&self.models, query);
        self.selected = 0;
        self.clamp_selected();
    }

    fn apply_filter_preserving_selection(&mut self, query: &str, preserve_id: Option<&str>) {
        self.filter_query = query.to_string();
        self.filtered = filter_model_indices(&self.models, query);
        if let Some(id) = preserve_id {
            self.selected = self
                .filtered
                .iter()
                .position(|index| self.models[*index].id == id)
                .unwrap_or(0);
        } else {
            self.selected = 0;
        }
        self.clamp_selected();
    }

    pub fn handle_key(
        &mut self,
        composer: &mut ComposerState,
        key: KeyEvent,
        content_width: usize,
    ) -> ModelAction {
        if !self.active || key.kind != KeyEventKind::Press {
            return ModelAction::None;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return self.cancel();
        }
        match key.code {
            KeyCode::Esc => self.cancel(),
            KeyCode::Up => {
                self.move_selected(-1);
                ModelAction::None
            }
            KeyCode::Down => {
                self.move_selected(1);
                ModelAction::None
            }
            KeyCode::Enter if !key.modifiers.contains(KeyModifiers::SHIFT) => self.select_current(),
            _ => {
                let before = composer.text().to_string();
                match composer.handle_key_with_width(key, content_width) {
                    ComposerAction::Quit => self.cancel(),
                    ComposerAction::Submit(_) => ModelAction::None,
                    _ => {
                        if composer.text() != before {
                            self.apply_filter(composer.text());
                        }
                        ModelAction::None
                    }
                }
            }
        }
    }

    pub fn insert_paste(&mut self, composer: &mut ComposerState, pasted: &str) {
        if !self.active {
            return;
        }
        composer.insert_paste(pasted);
        self.apply_filter(composer.text());
    }

    fn select_current(&mut self) -> ModelAction {
        if self.load_state == LoadState::Error {
            return self.cancel();
        }
        let Some(model_index) = self.filtered.get(self.selected).copied() else {
            return ModelAction::None;
        };
        let Some(model) = self.models.get(model_index) else {
            return ModelAction::None;
        };
        let result = match self.target {
            ModelTarget::Chat => save_default_model(&model.id, model.protocol.api_name()),
            ModelTarget::TextGeneration => save_text_generation_model(
                &model.metadata_provider_id,
                &model.id,
                model.protocol.api_name(),
            ),
        };
        match result {
            Ok(()) => {
                self.active = false;
                ModelAction::Selected {
                    target: self.target,
                }
            }
            Err(error) => ModelAction::Error { message: error },
        }
    }

    fn cancel(&mut self) -> ModelAction {
        self.active = false;
        ModelAction::Cancelled
    }

    fn replace_models(&mut self, models: Vec<ResolvedModel>, preserve_id: Option<&str>) {
        let preserve_id = preserve_id
            .map(str::to_string)
            .or_else(|| self.selected_model_id())
            .or_else(|| {
                if self.filter_query.is_empty() {
                    Some(self.current_model_id.clone())
                } else {
                    None
                }
            });
        self.models = Arc::new(models);
        let query = self.filter_query.clone();
        self.apply_filter_preserving_selection(&query, preserve_id.as_deref());
    }

    fn selected_model_id(&self) -> Option<String> {
        self.filtered
            .get(self.selected)
            .and_then(|index| self.models.get(*index))
            .map(|model| model.id.clone())
    }

    fn clamp_selected(&mut self) {
        if self.filtered.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.filtered.len() - 1);
        }
    }

    fn move_selected(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let len = self.filtered.len();
        if delta > 0 && self.selected + 1 >= len {
            self.selected = 0;
            return;
        }
        if delta < 0 && self.selected == 0 {
            self.selected = len - 1;
            return;
        }
        let last = len as isize - 1;
        let next = (self.selected as isize + delta).clamp(0, last);
        self.selected = next as usize;
    }

    fn spawn_refresh(&mut self, runtime: &Handle) {
        self.refresh = Some(runtime.spawn(async {
            refresh_connected_catalog_async().await
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::test_env_lock;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn typing_filters_models_without_leaving_picker() {
        let mut flow = ModelFlow {
            active: true,
            target: ModelTarget::Chat,
            models: Arc::new(vec![
                make_model("glm-5.2", "GLM-5.2"),
                make_model("gpt-5.6-luna", "GPT-5.6 Luna"),
            ]),
            filtered: vec![0, 1],
            selected: 0,
            filter_query: String::new(),
            current_model_id: "glm-5.2".into(),
            status: String::new(),
            load_state: LoadState::Ready,
            refresh: None,
        };
        let mut composer = ComposerState::default();
        flow.handle_key(&mut composer, key(KeyCode::Char('g')), 40);
        flow.handle_key(&mut composer, key(KeyCode::Char('p')), 40);
        flow.handle_key(&mut composer, key(KeyCode::Char('t')), 40);
        assert_eq!(flow.filtered().len(), 1);
        assert_eq!(flow.models()[flow.filtered()[0]].id, "gpt-5.6-luna");
    }

    #[test]
    fn navigation_wraps_at_both_ends() {
        let models = (0..5)
            .map(|index| make_model(&format!("model-{index}"), &format!("Model {index}")))
            .collect();
        let mut flow = ModelFlow {
            active: true,
            target: ModelTarget::Chat,
            models: Arc::new(models),
            filtered: vec![0, 1, 2, 3, 4],
            selected: 0,
            filter_query: String::new(),
            current_model_id: "model-0".into(),
            status: String::new(),
            load_state: LoadState::Ready,
            refresh: None,
        };
        let mut composer = ComposerState::default();

        flow.handle_key(&mut composer, key(KeyCode::Up), 40);
        assert_eq!(flow.selected(), 4);

        flow.handle_key(&mut composer, key(KeyCode::Down), 40);
        assert_eq!(flow.selected(), 0);
    }

    #[test]
    fn refresh_preserves_filter_and_selected_model() {
        let mut flow = ModelFlow {
            active: true,
            target: ModelTarget::Chat,
            models: Arc::new(vec![
                make_model("alpha", "Alpha"),
                make_model("beta", "Beta"),
                make_model("gamma", "Gamma"),
            ]),
            filtered: vec![1],
            selected: 0,
            filter_query: "be".into(),
            current_model_id: "alpha".into(),
            status: String::new(),
            load_state: LoadState::Ready,
            refresh: None,
        };

        flow.replace_models(
            vec![
                make_model("alpha", "Alpha"),
                make_model("beta", "Beta Next"),
                make_model("delta", "Delta"),
            ],
            Some("beta"),
        );

        assert_eq!(flow.filter_query, "be");
        assert_eq!(flow.filtered().len(), 1);
        assert_eq!(flow.models()[flow.filtered()[0]].id, "beta");
        assert_eq!(flow.models()[flow.filtered()[0]].name, "Beta Next");
        assert_eq!(flow.selected(), 0);
    }

    #[test]
    fn text_generation_selection_does_not_change_the_chat_model() {
        let _guard = test_env_lock();
        let dir = std::env::temp_dir().join(format!(
            "raid-text-model-flow-test-{}",
            std::process::id()
        ));
        unsafe {
            std::env::set_var("RAID_AGENT_DIR", &dir);
        }
        let mut settings = RaidSettings::default();
        settings.default_provider = Some("opencode-go".into());
        settings.default_model = Some("chat-model".into());
        settings.save().expect("save settings");

        let mut flow = ModelFlow {
            active: true,
            target: ModelTarget::TextGeneration,
            models: Arc::new(vec![make_model("title-model", "Title Model")]),
            filtered: vec![0],
            selected: 0,
            filter_query: String::new(),
            current_model_id: "chat-model".into(),
            status: String::new(),
            load_state: LoadState::Ready,
            refresh: None,
        };

        assert_eq!(
            flow.select_current(),
            ModelAction::Selected {
                target: ModelTarget::TextGeneration,
            }
        );
        let saved = RaidSettings::load();
        assert_eq!(saved.model_id(), "chat-model");
        assert_eq!(saved.text_generation_provider_id(), "opencode-go");
        assert_eq!(saved.text_generation_model_id(), "title-model");
        assert_eq!(saved.text_generation_api(), "openai-compatible");

        let _ = std::fs::remove_dir_all(&dir);
        unsafe {
            std::env::remove_var("RAID_AGENT_DIR");
        }
    }

    fn make_model(id: &str, name: &str) -> ResolvedModel {
        use crate::backend::opencode::types::{
            InterleavedFieldState, Modalities, ModelModality, ModelStatus, OpenCodePlan,
            OpenCodeProtocol, SdkPackage,
        };
        ResolvedModel {
            plan: OpenCodePlan::Go,
            plan_label: "Go".into(),
            metadata_provider_id: "opencode-go".into(),
            id: id.into(),
            name: name.into(),
            sdk_package: SdkPackage::OpenAiCompatible,
            protocol: OpenCodeProtocol::OpenAiCompatible,
            context_limit: 200_000,
            explicit_input_limit: None,
            output_limit: 32_000,
            tool_call: true,
            reasoning: true,
            modalities: Modalities {
                input: vec![ModelModality::Text],
                output: vec![ModelModality::Text],
            },
            interleaved: InterleavedFieldState::Unsupported { supported: false },
            cost: None,
            status: ModelStatus::Active,
            reasoning_variants: Vec::new(),
        }
    }
}
