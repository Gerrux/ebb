//! Library window: archive, trash, weekly review, import review, settings; the Sticky Notes
//! import panel.

pub struct Library {
    /// Settings: the language choice, before the language names.
    pub language: &'static str,
    /// Settings: follow the Windows UI language.
    pub language_system: &'static str,
}

pub const RU: Library = Library { language: "Язык", language_system: "Как в Windows" };

pub const EN: Library = Library { language: "Language", language_system: "Same as Windows" };
