//! Static subagent direction prompts.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DirectionId {
    General,
    Researcher,
    Explorer,
    Coder,
}

impl DirectionId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Researcher => "researcher",
            Self::Explorer => "explorer",
            Self::Coder => "coder",
        }
    }
}

const GENERAL_DIRECTION: &str = include_str!("general.md");
const RESEARCHER_DIRECTION: &str = include_str!("researcher.md");
const EXPLORER_DIRECTION: &str = include_str!("explorer.md");
const CODER_DIRECTION: &str = include_str!("coder.md");

pub fn direction_prompt(id: DirectionId) -> &'static str {
    match id {
        DirectionId::General => GENERAL_DIRECTION,
        DirectionId::Researcher => RESEARCHER_DIRECTION,
        DirectionId::Explorer => EXPLORER_DIRECTION,
        DirectionId::Coder => CODER_DIRECTION,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direction_prompts_are_non_empty() {
        assert!(!direction_prompt(DirectionId::General).trim().is_empty());
        assert!(!direction_prompt(DirectionId::Researcher).trim().is_empty());
        assert!(!direction_prompt(DirectionId::Explorer).trim().is_empty());
        assert!(!direction_prompt(DirectionId::Coder).trim().is_empty());
    }

    #[test]
    fn direction_id_as_str_is_stable() {
        assert_eq!(DirectionId::General.as_str(), "general");
        assert_eq!(DirectionId::Researcher.as_str(), "researcher");
        assert_eq!(DirectionId::Explorer.as_str(), "explorer");
        assert_eq!(DirectionId::Coder.as_str(), "coder");
    }
}
