use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize as _;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::WidgetRef;

use super::popup_consts::MAX_POPUP_ROWS;
use super::scroll_state::ScrollState;
use super::selection_popup_common::GenericDisplayRow;
use super::selection_popup_common::measure_rows_height;
use super::selection_popup_common::render_rows;
use crate::key_hint;
use crate::render::Insets;
use crate::render::RectExt;
use codex_common::fuzzy_match::fuzzy_match;
use codex_core::skills::model::SkillMetadata;
use crossterm::event::KeyCode;

pub(crate) struct SkillPopup {
    query: String,
    skills: Vec<SkillMetadata>,
    state: ScrollState,
}

impl SkillPopup {
    pub(crate) fn new(skills: Vec<SkillMetadata>) -> Self {
        Self {
            query: String::new(),
            skills,
            state: ScrollState::new(),
        }
    }

    pub(crate) fn set_skills(&mut self, skills: Vec<SkillMetadata>) {
        self.skills = skills;
        self.clamp_selection();
    }

    pub(crate) fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
        self.clamp_selection();
    }

    pub(crate) fn calculate_required_height(&self, width: u16) -> u16 {
        let rows = self.rows_from_matches(self.filtered());
        const HEADER_ROWS: u16 = 1;
        let content_width = width.saturating_sub(2).max(1);
        HEADER_ROWS + measure_rows_height(&rows, &self.state, MAX_POPUP_ROWS, content_width)
    }

    pub(crate) fn move_up(&mut self) {
        let len = self.filtered_items().len();
        self.state.move_up_wrap(len);
        self.state.ensure_visible(len, MAX_POPUP_ROWS.min(len));
    }

    pub(crate) fn move_down(&mut self) {
        let len = self.filtered_items().len();
        self.state.move_down_wrap(len);
        self.state.ensure_visible(len, MAX_POPUP_ROWS.min(len));
    }

    pub(crate) fn selected_skill(&self) -> Option<&SkillMetadata> {
        let matches = self.filtered_items();
        let idx = self.state.selected_idx?;
        let skill_idx = matches.get(idx)?;
        self.skills.get(*skill_idx)
    }

    fn clamp_selection(&mut self) {
        let len = self.filtered_items().len();
        self.state.clamp_selection(len);
        self.state.ensure_visible(len, MAX_POPUP_ROWS.min(len));
    }

    fn filtered_items(&self) -> Vec<usize> {
        self.filtered().into_iter().map(|(idx, _, _)| idx).collect()
    }

    fn rows_from_matches(
        &self,
        matches: Vec<(usize, Option<Vec<usize>>, i32)>,
    ) -> Vec<GenericDisplayRow> {
        matches
            .into_iter()
            .map(|(idx, indices, _score)| {
                let skill = &self.skills[idx];
                let slug = skill
                    .path
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or(&skill.name);
                let name = format!("{} ({slug})", skill.name);
                let description = skill.description.clone();
                GenericDisplayRow {
                    name,
                    match_indices: indices,
                    display_shortcut: None,
                    description: Some(description),
                    wrap_indent: None,
                }
            })
            .collect()
    }

    fn filtered(&self) -> Vec<(usize, Option<Vec<usize>>, i32)> {
        let filter = self.query.trim();
        let mut out: Vec<(usize, Option<Vec<usize>>, i32)> = Vec::new();

        if filter.is_empty() {
            for (idx, _skill) in self.skills.iter().enumerate() {
                out.push((idx, None, 0));
            }
            return out;
        }

        for (idx, skill) in self.skills.iter().enumerate() {
            if let Some((indices, score)) = fuzzy_match(&skill.name, filter) {
                out.push((idx, Some(indices), score));
            }
        }

        out.sort_by(|a, b| {
            a.2.cmp(&b.2).then_with(|| {
                let an = &self.skills[a.0].name;
                let bn = &self.skills[b.0].name;
                an.cmp(bn)
            })
        });

        out
    }
}

impl WidgetRef for SkillPopup {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        let inner = area.inset(Insets::tlbr(0, 2, 0, 0));
        if inner.is_empty() {
            return;
        }
        let header_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        };
        let list_area = Rect {
            x: inner.x,
            y: inner.y.saturating_add(1),
            width: inner.width,
            height: inner.height.saturating_sub(1),
        };

        let query = if self.query.is_empty() {
            "$".to_string()
        } else {
            format!("${}", self.query)
        };
        let header = Line::from(vec![
            Span::from(query).cyan().bold(),
            "  ·  ".dim(),
            key_hint::plain(KeyCode::Tab).into(),
            " insert".dim(),
            "  ·  ".dim(),
            key_hint::plain(KeyCode::Esc).into(),
            " close".dim(),
        ]);
        header.render_ref(header_area, buf);

        let rows = self.rows_from_matches(self.filtered());
        render_rows(
            list_area,
            buf,
            &rows,
            &self.state,
            MAX_POPUP_ROWS,
            "no skills",
        );
    }
}
