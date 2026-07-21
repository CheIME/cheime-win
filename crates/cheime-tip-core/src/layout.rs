//! Candidate layout computation — platform-independent, unit-testable.
//!
//! Computes row positions, column widths, highlight state, and hit-test
//! results from a `CandidateSnapshot` and font metrics. The actual GDI
//! rendering and HWND management live in `cheime-tip`.

use cheime_model::CandidateSnapshot;

/// Spacing constants (in pixels).
pub const ROW_PADDING_X: i32 = 8;
pub const ROW_PADDING_Y: i32 = 2;
pub const INDEX_WIDTH: i32 = 24;
pub const INDEX_PADDING: i32 = 4;

/// Describes how to render one row of the candidate window.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutRow {
    pub is_preedit: bool,
    pub y_offset: i32,
    pub text: String,
    pub index: Option<usize>, // 1-based display index
    pub annotation: Option<String>,
    pub is_highlighted: bool,
}

/// Layout result produced by `layout_snapshot`.
#[derive(Clone, Debug)]
pub struct CandidateLayout {
    pub preedit: String,
    pub preedit_cursor: usize,
    pub rows: Vec<LayoutRow>,
    pub total_width: i32,
    pub total_height: i32,
}

/// Compute the pixel size of the candidate window.
pub fn compute_window_size(
    snapshot: &CandidateSnapshot,
    line_height: i32,
    char_width: i32,
) -> (i32, i32) {
    let preedit_lines = if snapshot.preedit.is_empty() {
        0i32
    } else {
        1i32
    };
    let candidate_count = snapshot.candidates.len() as i32;

    if preedit_lines == 0 && candidate_count == 0 {
        return (0, 0);
    }

    let mut max_text_width = 0i32;
    if !snapshot.preedit.is_empty() {
        max_text_width = max_text_width.max(snapshot.preedit.len() as i32 * char_width);
    }
    for cand in &snapshot.candidates {
        let text_w = cand.text.len() as i32 * char_width;
        let ann_w = cand
            .annotation
            .as_ref()
            .map(|a| a.len() as i32 * char_width)
            .unwrap_or(0);
        max_text_width = max_text_width.max(INDEX_WIDTH + INDEX_PADDING + text_w + ann_w);
    }

    let width = max_text_width + ROW_PADDING_X * 2;
    let total_rows = preedit_lines + candidate_count;
    let height = total_rows * line_height + ROW_PADDING_Y * 2;
    (width, height)
}

/// Build a layout from a snapshot and metrics.
pub fn layout_snapshot(
    snapshot: &CandidateSnapshot,
    line_height: i32,
    char_width: i32,
) -> CandidateLayout {
    let (total_width, total_height) = compute_window_size(snapshot, line_height, char_width);
    let mut rows = Vec::new();
    let mut y = ROW_PADDING_Y;

    if !snapshot.preedit.is_empty() {
        rows.push(LayoutRow {
            is_preedit: true,
            y_offset: y,
            text: snapshot.preedit.clone(),
            index: None,
            annotation: None,
            is_highlighted: false,
        });
        y += line_height;
    }

    for (i, cand) in snapshot.candidates.iter().enumerate() {
        rows.push(LayoutRow {
            is_preedit: false,
            y_offset: y,
            text: cand.text.clone(),
            index: Some(i + 1),
            annotation: cand.annotation.clone(),
            is_highlighted: snapshot.highlighted == Some(cand.id),
        });
        y += line_height;
    }

    CandidateLayout {
        preedit: snapshot.preedit.clone(),
        preedit_cursor: snapshot.cursor,
        rows,
        total_width,
        total_height,
    }
}

/// Determine which candidate (1-based index) was clicked at `(x, y)`.
pub fn hit_test_candidate(
    layout: &CandidateLayout,
    _x: i32,
    y: i32,
    line_height: i32,
) -> Option<usize> {
    for row in &layout.rows {
        if y >= row.y_offset && y < row.y_offset + line_height && !row.is_preedit {
            return row.index;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use cheime_model::{
        Candidate, CandidateId, DeploymentGeneration, Revision, SessionEpoch, SessionStatus,
    };

    fn sample() -> CandidateSnapshot {
        CandidateSnapshot {
            epoch: SessionEpoch::new(1),
            revision: Revision::new(2),
            deployment: DeploymentGeneration::new(3),
            page: 0,
            page_size: 10,
            preedit: String::from("ni"),
            cursor: 2,
            candidates: vec![
                Candidate {
                    id: CandidateId::new(10),
                    text: String::from("你"),
                    annotation: Some(String::from("ni3")),
                    source: String::from("builtin"),
                    is_emoji: false,
                },
                Candidate {
                    id: CandidateId::new(11),
                    text: String::from("呢"),
                    annotation: None,
                    source: String::from("builtin"),
                    is_emoji: false,
                },
            ],
            highlighted: Some(CandidateId::new(10)),
            status: SessionStatus::Composing,
        }
    }

    #[test]
    fn layout_preedit_and_candidates() {
        let layout = layout_snapshot(&sample(), 22, 8);
        assert_eq!(layout.rows.len(), 3);
        assert!(layout.rows[0].is_preedit);
        assert_eq!(layout.rows[0].text, "ni");
        assert_eq!(layout.rows[1].text, "你");
        assert_eq!(layout.rows[1].index, Some(1));
        assert!(layout.rows[1].is_highlighted);
        assert!(!layout.rows[2].is_highlighted);
    }

    #[test]
    fn empty_snapshot_zero_size() {
        let snap = CandidateSnapshot {
            preedit: String::new(),
            cursor: 0,
            candidates: vec![],
            highlighted: None,
            status: SessionStatus::Ready,
            ..sample()
        };
        assert_eq!(compute_window_size(&snap, 22, 8), (0, 0));
    }

    #[test]
    fn hit_test_hits_candidate_misses_preedit() {
        let layout = layout_snapshot(&sample(), 22, 8);
        assert_eq!(hit_test_candidate(&layout, 20, 24, 22), Some(1)); // first candidate
        assert_eq!(hit_test_candidate(&layout, 20, 2, 22), None); // preedit row
    }
}
