use std::{
    borrow::Cow,
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

use anyhow::{Result, anyhow};
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use gpui::{App, AppContext, Entity, Hsla, Window};
use gpui_wry::WebView;
use lb_wry::{
    WebViewBuilder,
    http::{
        Request, Response, StatusCode,
        header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE},
    },
};
use raw_window_handle::HasWindowHandle;
use serde::Deserialize;

use super::cursor::CursorAsset;

const PLAYER_URL: &str = "recorder://player/index.html";
const VIDEO_URL_PATH: &str = "/recording.mp4";

#[derive(Clone, Copy, Debug)]
pub(super) enum PlaybackEvent {
    Time { seconds: f64, playing: bool },
    State(bool),
}

pub(super) fn build_webview(
    path: &Path,
    player_background: Hsla,
    cursor_asset: CursorAsset,
    window: &mut Window,
    cx: &mut App,
) -> Result<(Entity<WebView>, Receiver<PlaybackEvent>)> {
    if !path.is_file() {
        return Err(anyhow!("recording file does not exist: {}", path.display()));
    }

    let video_path = path.to_path_buf();
    let (event_sender, event_receiver) = bounded(1);
    let event_receiver_for_handler = event_receiver.clone();
    let builder = WebViewBuilder::new()
        .with_custom_protocol("recorder".to_string(), move |_, request| {
            serve_request(&video_path, player_background, cursor_asset, request)
        })
        .with_ipc_handler(move |request| {
            if let Some(event) = parse_browser_event(request.body()) {
                queue_event(&event_sender, &event_receiver_for_handler, event);
            }
        })
        .with_url(PLAYER_URL);
    let window_handle = window
        .window_handle()
        .map_err(|error| anyhow!("could not access playback window handle: {error}"))?;
    let webview = builder
        .build_as_child(&window_handle)
        .map_err(|error| anyhow!("could not create playback webview: {error}"))?;

    Ok((
        cx.new(|cx| WebView::new(webview, window, cx)),
        event_receiver,
    ))
}

fn serve_request(
    path: &Path,
    player_background: Hsla,
    cursor_asset: CursorAsset,
    request: Request<Vec<u8>>,
) -> Response<Cow<'static, [u8]>> {
    match request.uri().path() {
        "/" | "/index.html" => response(
            StatusCode::OK,
            "text/html; charset=utf-8",
            player_html(player_background, cursor_asset).into_bytes(),
        ),
        VIDEO_URL_PATH => serve_video(path, &request),
        _ => response(
            StatusCode::NOT_FOUND,
            "text/plain; charset=utf-8",
            Vec::new(),
        ),
    }
}

fn queue_event(
    sender: &Sender<PlaybackEvent>,
    receiver: &Receiver<PlaybackEvent>,
    mut event: PlaybackEvent,
) {
    loop {
        match sender.try_send(event) {
            Ok(()) => return,
            Err(TrySendError::Full(next)) => {
                let _ = receiver.try_recv();
                event = next;
            }
            Err(TrySendError::Disconnected(_)) => return,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum BrowserEvent {
    #[serde(rename = "time")]
    Time { time: f64, playing: bool },
    #[serde(rename = "state")]
    State { playing: bool },
}

fn parse_browser_event(body: &str) -> Option<PlaybackEvent> {
    match serde_json::from_str(body).ok()? {
        BrowserEvent::Time { time, playing } if time.is_finite() => Some(PlaybackEvent::Time {
            seconds: time.max(0.0),
            playing,
        }),
        BrowserEvent::State { playing } => Some(PlaybackEvent::State(playing)),
        BrowserEvent::Time { .. } => None,
    }
}

fn serve_video(path: &Path, request: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let length = match fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(_) => {
            return response(
                StatusCode::NOT_FOUND,
                "text/plain; charset=utf-8",
                Vec::new(),
            );
        }
    };
    let range = match parse_range(request.headers().get(RANGE), length) {
        Ok(range) => range,
        Err(()) => {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(CONTENT_RANGE, format!("bytes */{length}"))
                .body(Cow::Owned(Vec::new()))
                .expect("valid range response");
        }
    };
    let (start, end, partial) = match range {
        Some((start, end)) => (start, end, true),
        None if length == 0 => (0, 0, false),
        None => (0, length - 1, false),
    };
    let body = if length == 0 {
        Vec::new()
    } else {
        match read_segment(path, start, end) {
            Ok(body) => body,
            Err(_) => {
                return response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "text/plain; charset=utf-8",
                    Vec::new(),
                );
            }
        }
    };

    let status = if partial {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    let mut builder = Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "video/mp4")
        .header(ACCEPT_RANGES, "bytes")
        .header(CONTENT_LENGTH, body.len().to_string());
    if partial {
        builder = builder.header(CONTENT_RANGE, format!("bytes {start}-{end}/{length}"));
    }
    builder
        .body(Cow::Owned(body))
        .expect("valid video response")
}

fn parse_range(
    header: Option<&lb_wry::http::HeaderValue>,
    length: u64,
) -> std::result::Result<Option<(u64, u64)>, ()> {
    let Some(header) = header else {
        return Ok(None);
    };
    let value = header.to_str().map_err(|_| ())?;
    let value = value.strip_prefix("bytes=").ok_or(())?;
    let (start, end) = value.split_once('-').ok_or(())?;
    if length == 0 {
        return Err(());
    }

    if start.is_empty() {
        let suffix = end.parse::<u64>().map_err(|_| ())?;
        if suffix == 0 {
            return Err(());
        }
        return Ok(Some((length.saturating_sub(suffix), length - 1)));
    }

    let start = start.parse::<u64>().map_err(|_| ())?;
    let end = if end.is_empty() {
        length - 1
    } else {
        end.parse::<u64>().map_err(|_| ())?.min(length - 1)
    };
    if start >= length || start > end {
        return Err(());
    }

    Ok(Some((start, end)))
}

fn read_segment(path: &Path, start: u64, end: u64) -> io::Result<Vec<u8>> {
    let length = end
        .checked_sub(start)
        .and_then(|length| length.checked_add(1))
        .and_then(|length| usize::try_from(length).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "video segment is too large"))?;
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = vec![0; length];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn response(status: StatusCode, content_type: &str, body: Vec<u8>) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .header(CONTENT_LENGTH, body.len().to_string())
        .body(Cow::Owned(body))
        .expect("valid response")
}

fn player_html(player_background: Hsla, cursor_asset: CursorAsset) -> String {
    let cursor_svg = cursor_asset.svg();
    format!(
        r##"<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <style>
    html, body {{ width: 100%; height: 100%; }}
    html, body {{ margin: 0; overflow: hidden; background: #111214; }}
    #stage {{ position: relative; display: flex; align-items: center; justify-content: center; box-sizing: border-box; width: 100%; height: 100%; overflow: hidden; padding: 24px; background: #111214; }}
    #preview-background {{ position: absolute; inset: 24px; overflow: hidden; border-radius: 20px; background: linear-gradient(135deg, rgba(255,255,255,.08), rgba(255,255,255,0) 52%), {player_background}; }}
    #video-frame {{ position: relative; z-index: 1; width: 100%; height: 100%; max-width: 1440px; max-height: 810px; overflow: hidden; border: 1px solid rgba(255,255,255,.08); border-radius: 18px; background: transparent; box-shadow: 0 24px 70px rgba(0,0,0,.36); }}
    #video {{ position: absolute; inset: 0; display: block; width: 100%; height: 100%; object-fit: contain; background: transparent; }}
    #cursor-overlay {{ position: absolute; left: 0; top: 0; z-index: 2; display: none; width: 24px; height: 32px; pointer-events: none; transform-origin: 0 0; }}
    #cursor-overlay svg {{ display: block; width: 24px; height: 32px; }}
  </style>
</head>
<body>
  <div id="stage">
    <div id="preview-background" aria-hidden="true"></div>
    <div id="video-frame">
      <video id="video" controls preload="auto" playsinline src="/recording.mp4"></video>
      <div id="cursor-overlay" aria-hidden="true">{cursor_svg}</div>
    </div>
  </div>
  <script>
    const video = document.getElementById("video");
    const videoFrame = document.getElementById("video-frame");
    const cursor = document.getElementById("cursor-overlay");
    let framePending = false;

    function post(message) {{
      if (window.ipc) {{
        window.ipc.postMessage(JSON.stringify(message));
      }}
    }}

    function isPlaying() {{
      return !video.paused && !video.ended;
    }}

    function postTime() {{
      post({{ type: "time", time: video.currentTime, playing: isPlaying() }});
    }}

    function postState() {{
      post({{ type: "state", playing: isPlaying() }});
    }}

    function requestFrame() {{
      if (video.paused || video.ended || framePending) {{
        return;
      }}
      if (video.requestVideoFrameCallback) {{
        framePending = true;
        video.requestVideoFrameCallback(() => {{
          framePending = false;
          postTime();
          requestFrame();
        }});
      }}
    }}

    video.addEventListener("play", () => {{ postState(); postTime(); requestFrame(); }});
    video.addEventListener("pause", () => {{ postState(); postTime(); }});
    video.addEventListener("seeking", postTime);
    video.addEventListener("seeked", () => {{ postTime(); requestFrame(); }});
    video.addEventListener("timeupdate", postTime);
    video.addEventListener("loadedmetadata", () => {{ postState(); postTime(); }});
    video.addEventListener("ended", () => {{ postState(); postTime(); }});

    window.setCursorPosition = (x, y, visible, scale) => {{
      if (!visible || !Number.isFinite(x) || !Number.isFinite(y) || video.videoWidth <= 0 || video.videoHeight <= 0) {{
        cursor.style.display = "none";
        return;
      }}

      const frameWidth = videoFrame.clientWidth;
      const frameHeight = videoFrame.clientHeight;
      const videoScale = Math.min(frameWidth / video.videoWidth, frameHeight / video.videoHeight);
      const contentWidth = video.videoWidth * videoScale;
      const contentHeight = video.videoHeight * videoScale;
      const offsetX = (frameWidth - contentWidth) / 2;
      const offsetY = (frameHeight - contentHeight) / 2;
      const renderScale = scale;
      if (!Number.isFinite(renderScale) || renderScale <= 0) {{
        cursor.style.display = "none";
        return;
      }}

      cursor.style.display = "block";
      cursor.style.transform = "translate(" + (offsetX + x * contentWidth - 2 * renderScale) + "px, " + (offsetY + y * contentHeight - renderScale) + "px) scale(" + renderScale + ")";
    }};
  </script>
</body>
</html>"##
    )
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn parses_video_ranges() {
        let header = "bytes=10-19".parse().unwrap();
        assert_eq!(parse_range(Some(&header), 100), Ok(Some((10, 19))));
        let header = "bytes=-10".parse().unwrap();
        assert_eq!(parse_range(Some(&header), 100), Ok(Some((90, 99))));
    }

    #[test]
    fn clamps_open_ended_ranges() {
        let header = "bytes=90-".parse().unwrap();
        assert_eq!(parse_range(Some(&header), 100), Ok(Some((90, 99))));
    }
}
