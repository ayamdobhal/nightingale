use bevy::prelude::*;

#[derive(Component)]
pub struct SearchText;

#[derive(Component)]
pub struct StatsText;

#[derive(Component)]
pub struct AnalysisHint;

#[derive(Component)]
pub struct AnalyzeAllButton;

#[derive(Component)]
pub struct EmptyStateRoot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarAction {
    RescanFolder,
    ChangeFolder,
    SpotifySearch,
    Settings,
    ToggleTheme,
    Profile,
    Exit,
}

#[derive(Component)]
pub struct SidebarButton {
    pub action: SidebarAction,
}

#[derive(Component)]
pub struct ThemeToggleIcon;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsAction {
    ToggleFullscreen,
    SeparatorPrev,
    SeparatorNext,
    ModelPrev,
    ModelNext,
    BeamUp,
    BeamDown,
    BatchUp,
    BatchDown,
    ToggleShowLogs,
    DownloadFormatPrev,
    DownloadFormatNext,
    DownloadTimeoutDown,
    DownloadTimeoutUp,
    RestoreDefaults,
    Close,
}

#[derive(Component)]
pub struct SettingsOverlay;

#[derive(Component)]
pub struct SettingsButton {
    pub action: SettingsAction,
}

#[derive(Component)]
pub struct SettingsRow(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsField {
    Separator,
    Model,
    Beam,
    Batch,
    Fullscreen,
    ShowLogs,
    DownloadFormat,
    DownloadTimeout,
}

#[derive(Component)]
pub struct SettingsValueText(pub SettingsField);

#[derive(Component)]
pub struct ProfileOverlay;

#[derive(Component)]
pub struct ProfileNameInput;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileAction {
    Create,
    Switch(usize),
    Delete(usize),
    ConfirmDelete,
    CancelDelete,
    NewProfile,
    Close,
}

#[derive(Component)]
pub struct ProfileButton {
    pub action: ProfileAction,
}

#[derive(Component)]
pub struct ProfileLabelText;

#[derive(Component)]
pub struct ProfileNameLabel;

#[derive(Component)]
pub struct LanguagePickerOverlay;

#[derive(Component)]
pub struct LanguagePickerItem {
    pub lang_code: String,
    pub song_index: usize,
}

#[derive(Component)]
pub struct LanguagePickerClose;

#[derive(Component)]
pub struct AboutOverlay;

#[derive(Component)]
pub struct AboutCloseButton;

#[derive(Component)]
pub struct SortButton(pub super::LibrarySort);

// --- Spotify Search ---

#[derive(Component)]
pub struct SpotifySearchOverlay;

#[derive(Component)]
pub struct SpotifySearchInput;

#[derive(Component)]
pub struct SpotifySearchResultsRoot;

#[derive(Component)]
pub struct SpotifyDownloadQueueRoot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpotifySearchTab {
    Tracks,
    Albums,
}

#[derive(Component)]
pub struct SpotifyTabButton(pub SpotifySearchTab);

#[derive(Component)]
pub struct SpotifyTrackDownloadBtn {
    pub index: usize,
}

#[derive(Component)]
pub struct SpotifyAlbumDownloadBtn {
    pub index: usize,
}

#[derive(Component)]
pub struct SpotifyCloseButton;

#[derive(Resource)]
pub struct LanguagePickerTarget {
    #[allow(dead_code)]
    pub song_index: usize,
}
