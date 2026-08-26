use gpui::{AppContext, Context};

use super::model::RecorderState;
use super::playback;
use super::project::{self, SavedProject};
use super::ui::RecorderView;

impl RecorderView {
    pub(crate) fn open_latest_project(&mut self, cx: &mut Context<Self>) {
        if let Some(project) = self.projects.first().cloned() {
            self.open_project(project, cx);
        }
    }

    pub(crate) fn refresh_projects(&mut self, cx: &mut Context<Self>) {
        if self.state != RecorderState::Idle {
            return;
        }

        self.refresh_projects_in_background(cx, true);
    }

    pub(super) fn refresh_projects_in_background(
        &mut self,
        cx: &mut Context<Self>,
        update_status: bool,
    ) {
        self.project_refresh_generation = self.project_refresh_generation.wrapping_add(1);
        let generation = self.project_refresh_generation;
        if update_status {
            self.set_status("Refreshing projects…");
        }
        cx.notify();

        let view = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let projects = cx
                .background_spawn(async { project::load_projects() })
                .await;
            view.update(cx, |view, cx| {
                if view.state != RecorderState::Idle
                    || view.project_refresh_generation != generation
                {
                    return;
                }
                let count = projects.len();
                view.projects = projects;
                if update_status {
                    view.set_status(format!("{count} saved project(s)"));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn open_project(&mut self, project: SavedProject, cx: &mut Context<Self>) {
        if self.state != RecorderState::Idle {
            return;
        }

        let label = project.label();
        let video_path = project.video_path().to_path_buf();
        let telemetry_path = project.telemetry_path().to_path_buf();
        let metadata_path = project.metadata_path().to_path_buf();
        let settings_path = project.settings_path().to_path_buf();
        self.set_status(format!("Opening {label}…"));
        cx.notify();

        let settings_path_for_load = settings_path.clone();
        cx.spawn(async move |view, cx| {
            // Load settings from disk so reopened projects reflect every saved edit.
            let settings = cx
                .background_spawn(async move { project::load_settings(&settings_path_for_load) })
                .await;
            match playback::open(
                cx,
                video_path,
                telemetry_path,
                metadata_path,
                settings_path,
                settings,
                false,
                false,
            ) {
                Ok(_) => {
                    view.update(cx, |view, cx| {
                        view.set_status(format!("Opened {label}"));
                        cx.notify();
                    })
                    .ok();
                }
                Err(error) => {
                    view.update(cx, |view, cx| {
                        view.report_error(format!("Could not open project: {error}"), cx);
                    })
                    .ok();
                }
            }
        })
        .detach();
    }
}
