//! FFmpeg backend: the default on Linux/Windows, opt-in on macOS.

use std::path::{Path, PathBuf};
use std::sync::Once;

#[cfg(not(target_os = "macos"))]
use ffmpeg_next as ff;
#[cfg(target_os = "macos")]
use ffmpeg_next_macos as ff;

use super::{Decoder, RawFrame, VideoInfo};
use crate::image_data::MediaError;

fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        if let Err(err) = ff::init() {
            tracing::error!("FFmpeg failed to initialize: {err}");
        }
        ff::util::log::set_level(ff::util::log::Level::Error);
    });
}

fn error(path: &Path, reason: impl std::fmt::Display) -> MediaError {
    MediaError::Video {
        path: path.to_owned(),
        reason: reason.to_string(),
    }
}

pub(crate) fn probe(path: &Path) -> Result<VideoInfo, MediaError> {
    init();
    let input = ff::format::input(path).map_err(|e| error(path, e))?;
    let stream = input
        .streams()
        .best(ff::media::Type::Video)
        .ok_or_else(|| error(path, "no video stream"))?;
    let decoder = ff::codec::context::Context::from_parameters(stream.parameters())
        .and_then(|c| c.decoder().video())
        .map_err(|e| error(path, e))?;
    let rate = stream.avg_frame_rate();
    let frame_rate = if rate.denominator() != 0 {
        f64::from(rate)
    } else {
        0.0
    };
    let duration = if input.duration() > 0 {
        input.duration() as f64 / f64::from(ff::ffi::AV_TIME_BASE)
    } else if stream.duration() > 0 {
        stream.duration() as f64 * f64::from(stream.time_base())
    } else {
        0.0
    };
    if decoder.width() == 0 || decoder.height() == 0 {
        return Err(error(path, "video has no size"));
    }
    Ok(VideoInfo {
        width: decoder.width(),
        height: decoder.height(),
        duration,
        frame_rate,
    })
}

pub(crate) struct FfmpegDecoder {
    path: PathBuf,
    input: ff::format::context::Input,
    decoder: ff::decoder::Video,
    index: usize,
    time_base: f64,
    start: f64,
    scaler: Option<(ff::format::Pixel, u32, u32, ff::software::scaling::Context)>,
    decoded: ff::frame::Video,
    rgba: ff::frame::Video,
    eof_sent: bool,
}

impl Decoder for FfmpegDecoder {
    fn open(path: &Path) -> Result<Self, MediaError> {
        init();
        let err = |e: ff::Error| error(path, e);
        let input = ff::format::input(path).map_err(err)?;
        let stream = input
            .streams()
            .best(ff::media::Type::Video)
            .ok_or_else(|| error(path, "no video stream"))?;
        let index = stream.index();
        let time_base = f64::from(stream.time_base());
        let start = if stream.start_time() != ff::ffi::AV_NOPTS_VALUE {
            stream.start_time() as f64 * time_base
        } else {
            0.0
        };
        let mut context =
            ff::codec::context::Context::from_parameters(stream.parameters()).map_err(err)?;
        context.set_threading(ff::codec::threading::Config {
            kind: ff::codec::threading::Type::Frame,
            count: 0,
        });
        let decoder = context.decoder().video().map_err(err)?;
        Ok(Self {
            path: path.to_owned(),
            input,
            decoder,
            index,
            time_base,
            start,
            scaler: None,
            decoded: ff::frame::Video::empty(),
            rgba: ff::frame::Video::empty(),
            eof_sent: false,
        })
    }

    fn seek(&mut self, seconds: f64) -> Result<(), MediaError> {
        let ts = (seconds * f64::from(ff::ffi::AV_TIME_BASE)) as i64;
        self.input
            .seek(ts, ..ts + 1)
            .map_err(|e| error(&self.path, e))?;
        self.decoder.flush();
        self.eof_sent = false;
        Ok(())
    }

    fn next_frame(&mut self) -> Result<Option<RawFrame>, MediaError> {
        loop {
            if self.decoder.receive_frame(&mut self.decoded).is_ok() {
                return self.convert().map(Some);
            }
            if self.eof_sent {
                return Ok(None);
            }
            let mut packet = ff::Packet::empty();
            match packet.read(&mut self.input) {
                Ok(()) => {
                    if packet.stream() == self.index
                        && let Err(e) = self.decoder.send_packet(&packet)
                    {
                        tracing::debug!("decode error (skipping packet): {e}");
                    }
                }
                Err(ff::Error::Eof) => {
                    let _ = self.decoder.send_eof();
                    self.eof_sent = true;
                }
                Err(e) => tracing::debug!("packet read error (skipping): {e}"),
            }
        }
    }
}

impl FfmpegDecoder {
    fn convert(&mut self) -> Result<RawFrame, MediaError> {
        let err = |e: ff::Error| error(&self.path, e);
        let pts = self.decoded.timestamp().or(self.decoded.pts());
        let time = pts.map_or(0.0, |p| p as f64 * self.time_base - self.start);
        let (fmt, w, h) = (
            self.decoded.format(),
            self.decoded.width(),
            self.decoded.height(),
        );
        if self
            .scaler
            .as_ref()
            .is_none_or(|(f, sw, sh, _)| (*f, *sw, *sh) != (fmt, w, h))
        {
            let ctx = ff::software::scaling::Context::get(
                fmt,
                w,
                h,
                ff::format::Pixel::RGBA,
                w,
                h,
                ff::software::scaling::Flags::BILINEAR,
            )
            .map_err(err)?;
            self.scaler = Some((fmt, w, h, ctx));
        }
        let Some((_, _, _, sws)) = self.scaler.as_mut() else {
            return Err(error(&self.path, "no scaler"));
        };
        sws.run(&self.decoded, &mut self.rgba)
            .map_err(|e| error(&self.path, e))?;
        let stride = self.rgba.stride(0);
        Ok(RawFrame {
            time,
            width: w,
            height: h,
            stride: stride as u32,
            data: self.rgba.data(0)[..stride * h as usize].to_vec(),
        })
    }
}
