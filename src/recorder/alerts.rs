use std::{collections::VecDeque, rc::Rc};

use gpui::{
    AnyElement, App, ClickEvent, Hsla, IntoElement, ParentElement as _, SharedString, Styled as _,
    Window, div, px,
};
use gpui_component::{Sizable as _, Size, alert::Alert};

const MAX_PENDING_ALERTS: usize = 8;
const ALERT_WIDTH: f32 = 360.0;
const ALERT_OFFSET: f32 = 16.0;
pub(super) type AlertId = u64;
pub(super) type AlertCloseHandler = Rc<dyn Fn(AlertId, &mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AlertKind {
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AppAlert {
    id: AlertId,
    kind: AlertKind,
    message: SharedString,
}

pub(super) struct AlertQueue {
    alerts: VecDeque<AppAlert>,
    next_id: AlertId,
}

impl Default for AlertQueue {
    fn default() -> Self {
        Self {
            alerts: VecDeque::new(),
            next_id: 1,
        }
    }
}

impl AlertQueue {
    pub(super) fn push(&mut self, alert: AppAlert) {
        if self.alerts.back().is_some_and(|previous| {
            previous.kind == alert.kind && previous.message == alert.message
        }) {
            return;
        }
        if self.alerts.len() == MAX_PENDING_ALERTS {
            self.alerts.pop_front();
        }
        self.alerts.push_back(AppAlert {
            id: self.next_id,
            ..alert
        });
        self.next_id = self.next_id.wrapping_add(1);
    }

    pub(super) fn dismiss(&mut self, id: AlertId) -> bool {
        let before = self.alerts.len();
        self.alerts.retain(|alert| alert.id != id);
        self.alerts.len() != before
    }

    pub(super) fn is_empty(&self) -> bool {
        self.alerts.is_empty()
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &AppAlert> {
        self.alerts.iter()
    }
}

impl AppAlert {
    pub(super) fn error(message: impl Into<SharedString>) -> Self {
        Self {
            id: 0,
            kind: AlertKind::Error,
            message: message.into(),
        }
    }

    pub(super) fn warning(message: impl Into<SharedString>) -> Self {
        Self {
            id: 0,
            kind: AlertKind::Warning,
            message: message.into(),
        }
    }

    fn id(&self) -> AlertId {
        self.id
    }

    fn element(
        &self,
        on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        surface: Hsla,
        border: Hsla,
    ) -> Alert {
        let id = format!("recorder-alert-{}", self.id);
        match self.kind {
            AlertKind::Error => Alert::error(id, self.message.clone()),
            AlertKind::Warning => Alert::warning(id, self.message.clone()),
        }
        .with_size(Size::XSmall)
        .text_xs()
        .bg(surface)
        .border_color(border)
        .on_close(on_close)
    }
}

pub(super) fn render_layer(
    queue: &AlertQueue,
    on_close: AlertCloseHandler,
    surface: Hsla,
    border: Hsla,
) -> Option<AnyElement> {
    if queue.is_empty() {
        return None;
    }

    let alerts: Vec<_> = queue
        .iter()
        .map(|alert| {
            let id = alert.id();
            let on_close = on_close.clone();
            alert.element(
                move |_, window, cx| on_close(id, window, cx),
                surface,
                border,
            )
        })
        .collect();

    Some(
        div()
            .absolute()
            .top(px(ALERT_OFFSET))
            .right(px(ALERT_OFFSET))
            .w(px(ALERT_WIDTH))
            .flex()
            .flex_col()
            .gap_1()
            .children(alerts)
            .into_any_element(),
    )
}
