#[cfg(windows)]
mod recorder {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::path::Path;
    use std::thread;
    use std::time::Duration;

    use anyhow::anyhow;
    use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
    use gpui::*;
    use gpui_component::{
        ActiveTheme as _, Disableable, Root, Selectable, button::*, h_flex, v_flex,
    };
    use windows_capture::capture::{Context as CaptureContext, GraphicsCaptureApiHandler};
    use windows_capture::encoder::{
        AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
        VideoSettingsSubType,
    };
    use windows_capture::frame::Frame;
    use windows_capture::graphics_capture_api::InternalCaptureControl;
    use windows_capture::monitor::Monitor;
    use windows_capture::settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    };

    const OUTPUT_PATH: &str = "recording.mp4";
    const FRAME_INTERVAL: Duration = Duration::from_nanos(16_666_667);

    type CaptureError = Box<dyn std::error::Error + Send + Sync>;

    #[derive(Clone)]
    struct MonitorInfo {
        monitor: Monitor,
        label: SharedString,
        width: u32,
        height: u32,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum RecorderState {
        Idle,
        Starting,
        Recording,
        Stopping,
    }

    enum WorkerEvent {
        Started,
        Finished(Result<(), String>),
    }

    struct RecorderView {
        monitors: Vec<MonitorInfo>,
        selected_monitor: usize,
        state: RecorderState,
        status: SharedString,
        stop_sender: Option<Sender<()>>,
    }

    impl RecorderView {
        fn new(monitors: Result<Vec<MonitorInfo>, String>) -> Self {
            match monitors {
                Ok(monitors) => Self {
                    monitors,
                    selected_monitor: 0,
                    state: RecorderState::Idle,
                    status: "Ready to record".into(),
                    stop_sender: None,
                },
                Err(error) => Self {
                    monitors: Vec::new(),
                    selected_monitor: 0,
                    state: RecorderState::Idle,
                    status: format!("Monitor enumeration failed: {error}").into(),
                    stop_sender: None,
                },
            }
        }

        fn select_monitor(&mut self, index: usize, cx: &mut Context<Self>) {
            if self.state != RecorderState::Idle || index >= self.monitors.len() {
                return;
            }

            self.selected_monitor = index;
            let monitor = &self.monitors[index];
            self.status = format!("Ready: {} × {}", monitor.width, monitor.height).into();
            cx.notify();
        }

        fn start_recording(&mut self, cx: &mut Context<Self>) {
            if self.state != RecorderState::Idle {
                return;
            }

            let Some(monitor) = self.monitors.get(self.selected_monitor).cloned() else {
                self.status = "No monitor is available to record".into();
                cx.notify();
                return;
            };

            let (stop_sender, stop_receiver) = bounded(1);
            let (event_sender, event_receiver) = unbounded();

            self.stop_sender = Some(stop_sender);
            self.state = RecorderState::Starting;
            self.status =
                format!("Starting {} × {} capture…", monitor.width, monitor.height).into();

            spawn_capture_worker(
                monitor.monitor,
                monitor.width,
                monitor.height,
                stop_receiver,
                event_sender,
            );
            self.watch_worker(event_receiver, cx);
            cx.notify();
        }

        fn stop_recording(&mut self, cx: &mut Context<Self>) {
            if !matches!(
                self.state,
                RecorderState::Starting | RecorderState::Recording
            ) {
                return;
            }

            if let Some(sender) = &self.stop_sender {
                let _ = sender.send(());
                self.state = RecorderState::Stopping;
                self.status = "Finishing recording…".into();
                cx.notify();
            }
        }

        fn watch_worker(&self, events: Receiver<WorkerEvent>, cx: &mut Context<Self>) {
            cx.spawn(async move |view, cx| {
                loop {
                    let events = events.clone();
                    let event = cx.background_spawn(async move { events.recv().ok() }).await;
                    let Some(event) = event else {
                        break;
                    };

                    let finished = matches!(&event, WorkerEvent::Finished(_));
                    if view
                        .update(cx, |view, cx| view.apply_worker_event(event, cx))
                        .is_err()
                    {
                        break;
                    }

                    if finished {
                        break;
                    }
                }
            })
            .detach();
        }

        fn apply_worker_event(&mut self, event: WorkerEvent, cx: &mut Context<Self>) {
            match event {
                WorkerEvent::Started => {
                    if self.state == RecorderState::Starting {
                        self.state = RecorderState::Recording;
                        self.status = "Recording…".into();
                    }
                }
                WorkerEvent::Finished(result) => {
                    self.stop_sender = None;
                    self.state = RecorderState::Idle;
                    self.status = match result {
                        Ok(()) => format!("Saved {OUTPUT_PATH}").into(),
                        Err(error) => format!("Recording failed: {error}").into(),
                    };
                }
            }

            cx.notify();
        }
    }

    impl Render for RecorderView {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let can_select_monitor = self.state == RecorderState::Idle;
            let can_start = can_select_monitor && !self.monitors.is_empty();
            let can_stop = matches!(
                self.state,
                RecorderState::Starting | RecorderState::Recording
            );

            let monitor_buttons = self.monitors.iter().enumerate().map(|(index, monitor)| {
                Button::new(format!("monitor-{index}"))
                    .outline()
                    .selected(index == self.selected_monitor)
                    .disabled(!can_select_monitor)
                    .label(monitor.label.clone())
                    .on_click(cx.listener(move |view, _, _, cx| view.select_monitor(index, cx)))
            });

            let selected_dimensions = self
                .monitors
                .get(self.selected_monitor)
                .map(|monitor| format!("{} × {}", monitor.width, monitor.height))
                .unwrap_or_else(|| "Unavailable".to_string());

            div().size_full().bg(cx.theme().background).p_6().child(
                v_flex()
                    .gap_4()
                    .size_full()
                    .child(div().text_xl().child("Screen Recorder"))
                    .child(
                        v_flex()
                            .gap_2()
                            .child(
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Monitor"),
                            )
                            .child(h_flex().gap_2().children(monitor_buttons))
                            .child(
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("Selected resolution: {selected_dimensions}")),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("start-recording")
                                    .primary()
                                    .disabled(!can_start)
                                    .label("Start Recording")
                                    .on_click(
                                        cx.listener(|view, _, _, cx| view.start_recording(cx)),
                                    ),
                            )
                            .child(
                                Button::new("stop-recording")
                                    .danger()
                                    .disabled(!can_stop)
                                    .label("Stop Recording")
                                    .on_click(
                                        cx.listener(|view, _, _, cx| view.stop_recording(cx)),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.status.clone()),
                    )
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("Output: {OUTPUT_PATH}")),
                    ),
            )
        }
    }

    struct Capture {
        encoder: Option<VideoEncoder>,
    }

    impl Capture {
        fn finish(&mut self) -> Result<(), CaptureError> {
            if let Some(encoder) = self.encoder.take() {
                encoder.finish()?;
            }

            Ok(())
        }
    }

    impl GraphicsCaptureApiHandler for Capture {
        type Flags = (u32, u32);
        type Error = CaptureError;

        fn new(ctx: CaptureContext<Self::Flags>) -> Result<Self, Self::Error> {
            let encoder = VideoEncoder::new(
                VideoSettingsBuilder::new(ctx.flags.0, ctx.flags.1)
                    .sub_type(VideoSettingsSubType::H264)
                    .frame_rate(60),
                AudioSettingsBuilder::default().disabled(true),
                ContainerSettingsBuilder::default(),
                Path::new(OUTPUT_PATH),
            )?;

            Ok(Self {
                encoder: Some(encoder),
            })
        }

        fn on_frame_arrived(
            &mut self,
            frame: &mut Frame,
            _: InternalCaptureControl,
        ) -> Result<(), Self::Error> {
            self.encoder
                .as_mut()
                .ok_or_else(|| anyhow!("encoder was already finalized"))?
                .send_frame(frame)?;

            Ok(())
        }

        fn on_closed(&mut self) -> Result<(), Self::Error> {
            self.finish()
        }
    }

    fn enumerate_monitors() -> Result<Vec<MonitorInfo>, String> {
        let monitors = Monitor::enumerate().map_err(|error| error.to_string())?;
        if monitors.is_empty() {
            return Err("No monitors were found".to_string());
        }

        monitors
            .into_iter()
            .enumerate()
            .map(|(index, monitor)| {
                let width = monitor.width().map_err(|error| error.to_string())?;
                let height = monitor.height().map_err(|error| error.to_string())?;
                let name = monitor
                    .name()
                    .unwrap_or_else(|_| format!("Monitor {}", index + 1));

                Ok(MonitorInfo {
                    monitor,
                    label: format!("{name} — {width} × {height}").into(),
                    width,
                    height,
                })
            })
            .collect()
    }

    fn spawn_capture_worker(
        monitor: Monitor,
        width: u32,
        height: u32,
        stop_receiver: Receiver<()>,
        event_sender: Sender<WorkerEvent>,
    ) {
        thread::spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| {
                run_capture_worker(monitor, width, height, stop_receiver, &event_sender)
            }))
            .unwrap_or_else(|_| Err("capture worker panicked".to_string()));

            let _ = event_sender.send(WorkerEvent::Finished(result));
        });
    }

    fn run_capture_worker(
        monitor: Monitor,
        width: u32,
        height: u32,
        stop_receiver: Receiver<()>,
        event_sender: &Sender<WorkerEvent>,
    ) -> Result<(), String> {
        let settings = Settings::new(
            monitor,
            CursorCaptureSettings::Default,
            DrawBorderSettings::Default,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Custom(FRAME_INTERVAL),
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            (width, height),
        );

        let control = Capture::start_free_threaded(settings).map_err(|error| error.to_string())?;
        let callback = control.callback();
        let _ = event_sender.send(WorkerEvent::Started);

        let _ = stop_receiver.recv();

        let stop_error = control.stop().err().map(|error| error.to_string());
        let finish_error = callback
            .lock()
            .finish()
            .err()
            .map(|error| error.to_string());

        match stop_error.or(finish_error) {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub fn run() {
        let monitors = enumerate_monitors();
        let app = gpui_platform::application().with_assets(gpui_component_assets::Assets);

        app.run(move |cx| {
            gpui_component::init(cx);

            let window_options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(680.), px(400.)), cx)),
                ..Default::default()
            };

            cx.spawn(async move |cx| {
                cx.open_window(window_options, |window, cx| {
                    let view = cx.new(|_| RecorderView::new(monitors));
                    cx.new(|cx| Root::new(view, window, cx))
                })
                .expect("failed to open recorder window");
            })
            .detach();
        });
    }
}

fn main() {
    #[cfg(windows)]
    recorder::run();

    #[cfg(not(windows))]
    eprintln!("This screen recorder only runs on Windows.");
}
