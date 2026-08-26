use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{ActiveTheme as _, h_flex, scroll::ScrollableElement as _, v_flex};

use super::components::refresh_projects_button;
use super::project::SavedProject;
use super::ui::RecorderView;

pub(super) fn render_saved_projects(
    projects: &[SavedProject],
    enabled: bool,
    cx: &mut Context<RecorderView>,
) -> AnyElement {
    let list = if projects.is_empty() {
        v_flex()
            .flex_1()
            .min_h(px(48.))
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("No recordings yet."),
            )
            .into_any_element()
    } else {
        let last_index = projects.len() - 1;
        v_flex()
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .children(
                projects
                    .iter()
                    .enumerate()
                    .map(|(index, project)| project_row(index, last_index, project, enabled, cx)),
            )
            .into_any_element()
    };

    v_flex()
        .w_full()
        .flex_1()
        .min_h_0()
        .gap_2()
        .pt_3()
        .border_t_1()
        .border_color(cx.theme().border)
        .child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .gap_3()
                .child(section_header("Saved recordings", projects.len(), cx))
                .child(refresh_projects_button(enabled, cx)),
        )
        .child(list)
        .into_any_element()
}

fn project_row(
    index: usize,
    last_index: usize,
    project: &SavedProject,
    enabled: bool,
    cx: &mut Context<RecorderView>,
) -> AnyElement {
    let border = cx.theme().border;
    let hover_bg = cx.theme().muted;
    let project = project.clone();
    let open_project = project.clone();
    div()
        .id(format!("project-{index}"))
        .w_full()
        .px_2()
        .py_1p5()
        .when(index != last_index, |row| {
            row.border_b_1().border_color(border)
        })
        .when(enabled, |row| {
            row.cursor_pointer().hover(move |style| style.bg(hover_bg))
        })
        .when(!enabled, |row| row.opacity(0.65))
        .on_click(cx.listener(move |view, _, _, cx| view.open_project(open_project.clone(), cx)))
        .child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .gap_3()
                .min_w_0()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .truncate()
                                .child(format!("Recording {}", last_index + 1 - index)),
                        )
                        .child(summary_line(&project, cx)),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .whitespace_nowrap()
                        .child(relative_age(&project)),
                ),
        )
        .into_any_element()
}

fn summary_line(project: &SavedProject, cx: &Context<RecorderView>) -> AnyElement {
    let mut summary = project.source_summary().to_string();
    let (width, height) = project.dimensions();
    if width > 0 && height > 0 {
        summary.push_str(&format!(" · {width} × {height}"));
    }

    div()
        .min_w_0()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .whitespace_nowrap()
        .truncate()
        .child(summary)
        .into_any_element()
}

/// Newest recordings get the highest number, matching the list order.
fn relative_age(project: &SavedProject) -> String {
    let created = project.created_at_epoch().unwrap_or_default();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(created);
    format_age(now.saturating_sub(created))
}

fn format_age(age_secs: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;

    match age_secs {
        0..=44 => "just now".to_string(),
        45..=89 => "1 min ago".to_string(),
        age if age < HOUR => format!("{} min ago", age / MINUTE),
        age if age < HOUR + HOUR / 2 => "1 hr ago".to_string(),
        age if age < DAY => format!("{} hr ago", age / HOUR),
        age if age < 2 * DAY => "yesterday".to_string(),
        age => format!("{} d ago", age / DAY),
    }
}

fn section_header(text: &'static str, count: usize, cx: &Context<RecorderView>) -> AnyElement {
    h_flex()
        .items_baseline()
        .gap_2()
        .min_w_0()
        .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(text))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(count.to_string()),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::format_age;

    #[test]
    fn formats_relative_ages() {
        assert_eq!(format_age(10), "just now");
        assert_eq!(format_age(60), "1 min ago");
        assert_eq!(format_age(2 * 60), "2 min ago");
        assert_eq!(format_age(90 * 60), "1 hr ago");
        assert_eq!(format_age(5 * 3600), "5 hr ago");
        assert_eq!(format_age(30 * 86400), "30 d ago");
    }
}
