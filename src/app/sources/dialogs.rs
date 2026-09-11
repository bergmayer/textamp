//! Library-specific forms. Drafts are not configuration; checking is read-only.
use super::*;
use crate::util::SecretString;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

#[derive(Debug, Clone)]
pub enum Dialog {
    Library(super::options::Options),
    Add { selected: usize },
    Navidrome(ServerForm),
    AudioMuse(super::audiomuse::Form),

    Webdav(WebdavForm),
}

#[derive(Debug, Clone, Default)]
pub struct Field {
    pub value: SecretString,
    /// UTF-8 byte boundary, not a screen column.
    pub cursor: usize,
    pub selected: bool,
}
impl Field {
    pub(super) fn new(value: impl Into<SecretString>) -> Self {
        let value = value.into();
        Self {
            cursor: value.len(),
            value,
            selected: false,
        }
    }
    pub(super) fn edit(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if key.code == KeyCode::Char('a') {
                self.selected = true;
            }
            return;
        }
        if key
            .modifiers
            .intersects(KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            return;
        }
        let previous = self.value[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i);
        let next = self.cursor
            + self.value[self.cursor..]
                .chars()
                .next()
                .map_or(0, char::len_utf8);
        match key.code {
            KeyCode::Left => self.cursor = previous,
            KeyCode::Right => self.cursor = next,
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.value.len(),
            KeyCode::Backspace | KeyCode::Delete if self.selected => {
                self.value = Default::default();
                self.cursor = 0;
            }
            KeyCode::Backspace => {
                self.value.replace_range(previous..self.cursor, "");
                self.cursor = previous;
            }
            KeyCode::Delete => {
                self.value.replace_range(self.cursor..next, "");
            }
            KeyCode::Char(c) if !c.is_control() && self.value.len() < 4096 => {
                if self.selected {
                    self.value = Default::default();
                    self.cursor = 0;
                }
                self.value.insert(self.cursor, c);
                self.cursor += c.len_utf8();
            }
            _ => return,
        }
        self.selected = false;
    }
}

#[derive(Debug, Clone)]
pub struct WebdavForm {
    pub instance: String,
    pub source_id: String,
    pub editing: bool,
    pub password_env: Option<String>,
    /// URL, username, password, optional display name.
    pub fields: [Field; 4],
    /// Fields 0–3, Save/Add 4, Cancel 5.
    pub focus: usize,
    pub error: Option<String>,
    pub task: Option<TaskLease>,
}
impl WebdavForm {
    pub const LABELS: [&'static str; 4] = ["URL", "Username", "Password", "Name (optional)"];
    pub fn new(source: Option<&FolderSource>) -> Self {
        let mut form = Self {
            instance: uuid::Uuid::new_v4().to_string(),
            source_id: source
                .map(|s| s.id.clone())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            editing: source.is_some(),
            password_env: None,
            fields: Default::default(),
            focus: 0,
            error: None,
            task: None,
        };
        if let Some(FolderSource {
            name,
            location:
                FolderLocation::Webdav {
                    url,
                    username,
                    password_env,
                },
            ..
        }) = source
        {
            form.password_env = password_env.clone();
            form.fields = [
                Field::new(url.as_str()),
                Field::new(username.as_deref().unwrap_or("")),
                Field::default(),
                Field::new(name.as_str()),
            ];
        }
        form
    }
    pub fn from_location(source: FolderSource) -> Self {
        let mut form = Self::new(Some(&source));
        form.editing = false;
        form
    }
    pub fn draft(&self) -> anyhow::Result<WebdavDraft> {
        let mut source = super::source_from_location(self.fields[0].value.trim())?;
        let FolderLocation::Webdav {
            url,
            username,
            password_env,
        } = &mut source.location
        else {
            anyhow::bail!("Enter a WebDAV URL beginning with http:// or https://");
        };
        let original = reqwest::Url::parse(self.fields[0].value.trim())?;
        anyhow::ensure!(
            original.username().is_empty(),
            "Use the separate username and password fields, not credentials in the URL"
        );
        *username = (!self.fields[1].value.trim().is_empty())
            .then(|| self.fields[1].value.trim().to_owned());
        let keep = self.editing && self.fields[2].value.is_empty() && username.is_some();
        let password = if username.is_none() {
            Some(SecretString::default())
        } else {
            (!keep).then(|| self.fields[2].value.clone())
        };
        *password_env = if keep {
            self.password_env.clone()
        } else {
            None
        };
        if !url.ends_with('/') {
            url.push('/');
        }
        source.id = self.source_id.clone();
        if !self.fields[3].value.trim().is_empty() {
            source.name = self.fields[3].value.trim().to_owned();
        }
        source.validate()?;
        Ok(WebdavDraft {
            instance: self.instance.clone(),
            source,
            password,
        })
    }
}

#[derive(Debug, Clone)]
pub struct WebdavDraft {
    pub instance: String,
    pub source: FolderSource,
    /// None retains the stored password when editing; a value replaces it.
    pub password: Option<SecretString>,
}

#[derive(Debug, Clone)]
pub struct ServerForm {
    pub source: crate::navidrome::Source,
    pub fields: [Field; 4],
    pub focus: usize,
    pub error: Option<String>,
    pub busy: bool,
    pub editing: bool,
}
impl ServerForm {
    pub fn new(source: Option<&crate::navidrome::Source>) -> Self {
        let editing = source.is_some();
        let source = source.cloned().unwrap_or(crate::navidrome::Source {
            id: uuid::Uuid::new_v4().to_string(),
            name: String::new(),
            url: String::new(),
            username: String::new(),
            libraries: vec![],
        });
        let fields = [
            Field::new(source.url.as_str()),
            Field::new(source.username.as_str()),
            Field::default(),
            Field::new(source.name.as_str()),
        ];
        Self {
            source,
            fields,
            focus: 0,
            error: None,
            busy: false,
            editing,
        }
    }
    fn submit(&self) -> anyhow::Result<Action> {
        let mut source = self.source.clone();
        source.url = self.fields[0].value.trim().trim_end_matches('/').to_owned();
        source.username = self.fields[1].value.trim().to_owned();
        source.name = self.fields[3].value.trim().to_owned();
        if source.name.is_empty() {
            source.name = format!("{} @ {}", source.username, source.url);
        }
        source.validate()?;
        Ok(navidrome::NavAction::Password {
            source,
            password: self.fields[2].value.clone(),
        }
        .into())
    }
}
fn close(state: &mut AppState) {
    if matches!(&state.popups.library_dialog, Some(Dialog::Navidrome(form)) if form.busy) {
        state.sources.nav_connection_task = None;
        state.advance_connection_generation();
    }
    state.popups.library_dialog = None;
}

pub fn key(key: KeyEvent, state: &mut AppState) -> Vec<Action> {
    if key.code == KeyCode::Esc {
        close(state); // cancels the request and drops draft secrets
        return vec![];
    }
    if key.code == KeyCode::Char('q')
        && key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER)
    {
        return vec![super::super::action::SystemAction::Quit.into()];
    }
    let option_count = match &state.popups.library_dialog {
        Some(Dialog::Library(options)) => options.items(state).len(),
        _ => 0,
    };
    let Some(dialog) = &mut state.popups.library_dialog else {
        return vec![];
    };
    match dialog {
        Dialog::Library(options) => match key.code {
            KeyCode::Up | KeyCode::BackTab => options.selected = options.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab => {
                options.selected = (options.selected + 1).min(option_count.saturating_sub(1))
            }
            KeyCode::Home => options.selected = 0,
            KeyCode::End => options.selected = option_count.saturating_sub(1),
            KeyCode::Enter | KeyCode::Char(' ') => return super::options::activate(state),
            KeyCode::Left => close(state),
            KeyCode::F(5) => {
                return vec![crate::app::action::SettingsAction::RescanSourceCache(
                    options.choice.clone(),
                )
                .into()]
            }
            _ => {}
        },
        Dialog::Navidrome(form) => {
            if form.busy {
                return vec![];
            }
            match key.code {
                KeyCode::Tab | KeyCode::Down => form.focus = (form.focus + 1) % 6,
                KeyCode::BackTab | KeyCode::Up => form.focus = (form.focus + 5) % 6,
                KeyCode::Left | KeyCode::Right if form.focus >= 4 => form.focus = 9 - form.focus,
                KeyCode::Enter => return activate(state),
                _ if form.focus < 4 => form.fields[form.focus].edit(key),
                _ => {}
            }
        }
        Dialog::Add { selected } => match key.code {
            KeyCode::Up | KeyCode::BackTab => *selected = selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab => *selected = (*selected + 1).min(2),
            KeyCode::Enter => return activate(state),
            _ => {}
        },
        Dialog::AudioMuse(form) => {
            if form.task.is_some() {
                return vec![];
            }
            match key.code {
                KeyCode::Tab | KeyCode::Down => form.focus = (form.focus + 1) % 7,
                KeyCode::BackTab | KeyCode::Up => form.focus = (form.focus + 6) % 7,
                KeyCode::Left if form.focus >= 4 => {
                    form.focus = if form.focus == 4 { 6 } else { form.focus - 1 }
                }
                KeyCode::Right if form.focus >= 4 => {
                    form.focus = if form.focus == 6 { 4 } else { form.focus + 1 }
                }
                KeyCode::Enter => return activate(state),
                _ if form.focus < 4 => form.fields[form.focus].edit(key),
                _ => {}
            }
        }
        Dialog::Webdav(form) => {
            if form.task.is_some() {
                if key.code == KeyCode::Enter {
                    state.popups.library_dialog = None;
                }
                return vec![];
            }
            match key.code {
                KeyCode::Tab | KeyCode::Down => form.focus = (form.focus + 1) % 6,
                KeyCode::BackTab | KeyCode::Up => form.focus = (form.focus + 5) % 6,
                KeyCode::Left | KeyCode::Right if form.focus >= 4 => form.focus = 9 - form.focus,
                KeyCode::Enter => return activate(state),
                _ if form.focus < 4 => form.fields[form.focus].edit(key),
                _ => {}
            }
        }
    }
    vec![]
}

pub fn mouse(event: MouseEvent, state: &mut AppState) -> Vec<Action> {
    if event.kind != MouseEventKind::Down(MouseButton::Left) {
        return vec![];
    }
    let Some(index) = state
        .hit_regions
        .library_dialog
        .iter()
        .position(|rect| rect.contains((event.column, event.row).into()))
    else {
        return vec![];
    };
    match &mut state.popups.library_dialog {
        Some(Dialog::Library(options)) => {
            options.selected = index;
            super::options::activate(state)
        }
        Some(Dialog::Navidrome(form)) => {
            if index == 5 {
                close(state);
                return vec![];
            }
            if form.busy {
                return vec![];
            }
            form.focus = index;
            if index < 4 {
                form.fields[index].cursor = form.fields[index].value.len();
                vec![]
            } else {
                activate(state)
            }
        }
        Some(Dialog::Add { selected }) => {
            *selected = index;
            activate(state)
        }
        Some(Dialog::AudioMuse(form)) => {
            if form.task.is_some() {
                if index == 6 {
                    state.popups.library_dialog = None;
                }
                return vec![];
            }
            form.focus = index;
            if index < 4 {
                form.fields[index].cursor = form.fields[index].value.len();
                vec![]
            } else {
                activate(state)
            }
        }
        Some(Dialog::Webdav(form)) => {
            if form.task.is_some() {
                if index == 5 {
                    state.popups.library_dialog = None;
                }
                return vec![];
            }
            form.focus = index;
            if index < 4 {
                form.fields[index].cursor = form.fields[index].value.len();
                vec![]
            } else {
                activate(state)
            }
        }
        _ => vec![],
    }
}

fn activate(state: &mut AppState) -> Vec<Action> {
    match state.popups.library_dialog.as_mut() {
        Some(Dialog::Navidrome(form)) => {
            if form.focus == 5 {
                close(state);
                return vec![];
            }
            match form.submit() {
                Ok(action) => vec![action],
                Err(error) => {
                    form.error = Some(error.to_string());
                    vec![]
                }
            }
        }
        Some(Dialog::Add { selected }) => {
            let entry = [
                LibraryChoice::AddFolder,
                LibraryChoice::AddWebdav,
                LibraryChoice::AddNavidrome,
            ]
            .get(*selected)
            .cloned();
            state.popups.library_dialog = None;
            entry.map(|e| vec![e.action()]).unwrap_or_default()
        }
        Some(Dialog::AudioMuse(form)) => match form.focus {
            6 => {
                state.popups.library_dialog = None;
                vec![]
            }
            5 => vec![super::audiomuse::Command::Disconnect.into()],
            _ => vec![super::audiomuse::Command::Submit.into()],
        },
        Some(Dialog::Webdav(form)) if form.focus == 5 => {
            state.popups.library_dialog = None;
            vec![]
        }
        Some(Dialog::Webdav(form)) if form.task.is_none() => match form.draft() {
            Ok(draft) => vec![SourceAction::CheckWebdav(draft).into()],
            Err(error) => {
                form.error = Some(error.to_string());
                vec![]
            }
        },
        _ => vec![],
    }
}

pub async fn dispatch(
    action: SourceAction,
    state: &mut AppState,

    audio: &mut AudioPlayer,
    config: &mut Config,
    tx: &mpsc::Sender<Event>,
) -> anyhow::Result<Vec<Action>> {
    match action {
        SourceAction::CheckWebdav(mut draft) => {
            let Some(Dialog::Webdav(form)) = &mut state.popups.library_dialog else {
                return Ok(vec![]);
            };
            if form.instance != draft.instance || form.task.is_some() {
                return Ok(vec![]);
            }
            if config
                .folder_sources
                .iter()
                .any(|source| source.id != draft.source.id && source.same_library(&draft.source))
            {
                form.error = Some("That WebDAV library is already added".into());
                return Ok(vec![]);
            }
            form.error = None;
            form.focus = 5; // Cancel is the only enabled button during the check.
            let tx = tx.clone();
            let instance = draft.instance.clone();
            let task = crate::app::tasks::spawn(async move {
                let result = async {
                    if draft.password.is_none() {
                        let id = draft.source.id.clone();
                        let env = match &draft.source.location {
                            FolderLocation::Webdav { password_env, .. } => password_env.clone(),
                            _ => None,
                        };
                        draft.password = Some(
                            crate::app::tasks::spawn_blocking(move || {
                                crate::library::source_secret(&id, &env)
                            })
                            .await??
                            .map(|s| SecretString::from(s.as_str()))
                            .unwrap_or_default(),
                        );
                    }
                    tokio::time::timeout(
                        std::time::Duration::from_secs(30),
                        crate::library::check_webdav(
                            &draft.source,
                            draft.password.as_deref().map(String::as_str).unwrap_or(""),
                        ),
                    )
                    .await
                    .map_err(|_| anyhow::anyhow!("WebDAV connection timed out"))??;
                    Ok(draft)
                }
                .await
                .map_err(|error: anyhow::Error| format!("{error:#}"));
                let _ = tx
                    .send(Event::Effect(
                        SourceAction::WebdavChecked { instance, result }.into(),
                    ))
                    .await;
            });
            form.task = Some(TaskLease::new(&task));
        }
        SourceAction::WebdavChecked { instance, result } => {
            let Some(Dialog::Webdav(form)) = &mut state.popups.library_dialog else {
                return Ok(vec![]);
            };
            if form.instance != instance {
                return Ok(vec![]);
            }
            form.task = None;
            form.focus = 0;
            let mut draft = match result {
                Ok(draft) => draft,
                Err(error) => {
                    form.error = Some(error);
                    return Ok(vec![]);
                }
            };
            // Recheck after the asynchronous request, before writing secrets or config.
            if config
                .folder_sources
                .iter()
                .any(|source| source.id != draft.source.id && source.same_library(&draft.source))
            {
                form.error = Some("That WebDAV library is already added".into());
                return Ok(vec![]);
            }
            if let FolderLocation::Webdav { password_env, .. } = &mut draft.source.location {
                *password_env = None;
            }
            let reload = state.sources.active.folder() == Some(&draft.source.id)
                && config
                    .folder_sources
                    .iter()
                    .any(|s| s.id == draft.source.id && s.location != draft.source.location);
            let id = draft.source.id.clone();
            let password = draft.password.take().unwrap_or_default();
            if let Err(error) = crate::app::tasks::spawn_blocking(move || {
                crate::library::credentials::save(&id, password)
            })
            .await?
            {
                form.error = Some(format!("Save credentials: {error}"));
                return Ok(vec![]);
            }
            for choice in library_choices(state)
                .into_iter()
                .filter(|c| matches!(c, LibraryChoice::Folder(s) if s.id == draft.source.id))
            {
                super::cache::cancel(&choice, state);
            }
            config.folder_sources.retain(|s| s.id != draft.source.id);
            config.folder_sources.push(draft.source.clone());
            state.sources.folders = config.folder_sources.clone();
            state.popups.library_dialog = None;
            manager::reset_selection(state);
            state.popups.library_picker_index = choices(state)
                .iter()
                .position(|c| matches!(c, LibraryChoice::Folder(s) if s.id == draft.source.id))
                .unwrap_or(0);
            dispatch_settings::save_config_in_background(tx, config, "save WebDAV library");
            state.set_status("WebDAV library saved".into());
            if reload {
                super::reset_source(state, audio, tx).await?;
                return Ok(vec![LibraryChoice::Folder(draft.source).action()]);
            }
        }
        _ => unreachable!(),
    }
    Ok(vec![])
}
