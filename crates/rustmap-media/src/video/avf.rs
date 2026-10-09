#![allow(unsafe_code)] // FFI to AVFoundation/CoreMedia/CoreVideo; every block is commented.

//! AVFoundation backend (macOS): `AVAssetReader` decodes through VideoToolbox,
//! so H.264, HEVC and ProRes use the Mac's hardware decoders. Nothing to
//! install or bundle.

use std::path::{Path, PathBuf};

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2_av_foundation::{
    AVAsset, AVAssetReader, AVAssetReaderStatus, AVAssetReaderTrackOutput, AVAssetTrack,
    AVMediaTypeVideo, AVURLAsset,
};
use objc2_core_foundation::CFString;
use objc2_core_media::{CMTime, CMTimeRange, kCMTimePositiveInfinity};
use objc2_core_video::{
    CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight,
    CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
    CVPixelBufferUnlockBaseAddress, kCVPixelBufferPixelFormatTypeKey, kCVPixelFormatType_32BGRA,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};

use super::{Decoder, RawFrame, VideoInfo};
use crate::image_data::MediaError;

fn error(path: &Path, reason: impl std::fmt::Display) -> MediaError {
    MediaError::Video {
        path: path.to_owned(),
        reason: reason.to_string(),
    }
}

fn seconds(t: CMTime) -> f64 {
    // SAFETY: plain value conversion.
    let s = unsafe { t.seconds() };
    if s.is_finite() { s } else { 0.0 }
}

fn open_asset(path: &Path) -> Result<(Retained<AVURLAsset>, Retained<AVAssetTrack>), MediaError> {
    let path_str = path
        .to_str()
        .ok_or_else(|| error(path, "path is not valid UTF-8"))?;
    if !path.is_file() {
        return Err(error(path, "file not found"));
    }
    // SAFETY: standard AVFoundation calls with valid, retained arguments.
    unsafe {
        let url = NSURL::fileURLWithPath(&NSString::from_str(path_str));
        let asset = AVURLAsset::URLAssetWithURL_options(&url, None);
        let media_type =
            AVMediaTypeVideo.ok_or_else(|| error(path, "AVMediaTypeVideo unavailable"))?;
        #[allow(deprecated)]
        // the async loader needs a run loop; the sync call is fine on a worker thread
        let track = asset
            .tracksWithMediaType(media_type)
            .firstObject()
            .ok_or_else(|| error(path, "no video track (or a format macOS can't play)"))?;
        Ok((asset, track))
    }
}

pub(crate) fn probe(path: &Path) -> Result<VideoInfo, MediaError> {
    autoreleasepool(|_| {
        let (asset, track) = open_asset(path)?;
        // SAFETY: property getters on valid objects.
        let (size, rate, duration) = unsafe {
            (
                track.naturalSize(),
                track.nominalFrameRate(),
                asset.duration(),
            )
        };
        if size.width < 1.0 || size.height < 1.0 {
            return Err(error(path, "video has no size"));
        }
        Ok(VideoInfo {
            width: size.width.round() as u32,
            height: size.height.round() as u32,
            duration: seconds(duration),
            frame_rate: f64::from(rate),
        })
    })
}

pub(crate) struct AvfDecoder {
    path: PathBuf,
    asset: Retained<AVURLAsset>,
    track: Retained<AVAssetTrack>,
    reader: Retained<AVAssetReader>,
    output: Retained<AVAssetReaderTrackOutput>,
}

impl AvfDecoder {
    /// A reader that decodes to BGRA starting at `start` seconds.
    fn make_reader(
        path: &Path,
        asset: &AVAsset,
        track: &AVAssetTrack,
        start: f64,
    ) -> Result<(Retained<AVAssetReader>, Retained<AVAssetReaderTrackOutput>), MediaError> {
        // SAFETY: standard AVFoundation reader setup; the dictionary key is a
        // CFString constant, toll-free bridged to NSString.
        unsafe {
            let reader = AVAssetReader::assetReaderWithAsset_error(asset)
                .map_err(|e| error(path, e.localizedDescription()))?;
            let key: &CFString = kCVPixelBufferPixelFormatTypeKey;
            let key: &NSString = &*(key as *const CFString).cast::<NSString>();
            let format = NSNumber::new_u32(kCVPixelFormatType_32BGRA);
            let value: &AnyObject = &format;
            let settings = NSDictionary::<NSString, AnyObject>::from_slices(&[key], &[value]);
            let output = AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(
                track,
                Some(&settings),
            );
            // We copy each frame out right away, so let AVFoundation skip its own copy.
            output.setAlwaysCopiesSampleData(false);
            if !reader.canAddOutput(&output) {
                return Err(error(path, "AVAssetReader cannot decode this track"));
            }
            reader.addOutput(&output);
            if start > 0.0 {
                reader.setTimeRange(CMTimeRange {
                    start: CMTime::with_seconds(start, 600),
                    duration: kCMTimePositiveInfinity,
                });
            }
            if !reader.startReading() {
                let reason = reader.error().map_or_else(
                    || "could not start reading".to_owned(),
                    |e| e.localizedDescription().to_string(),
                );
                return Err(error(path, reason));
            }
            Ok((reader, output))
        }
    }
}

impl Decoder for AvfDecoder {
    fn open(path: &Path) -> Result<Self, MediaError> {
        autoreleasepool(|_| {
            let (asset, track) = open_asset(path)?;
            let (reader, output) = Self::make_reader(path, &asset, &track, 0.0)?;
            Ok(Self {
                path: path.to_owned(),
                asset,
                track,
                reader,
                output,
            })
        })
    }

    fn seek(&mut self, seconds: f64) -> Result<(), MediaError> {
        // AVAssetReader can't seek; start a new one at the target.
        autoreleasepool(|_| {
            // SAFETY: cancelling a reader we own.
            unsafe { self.reader.cancelReading() };
            let (reader, output) =
                Self::make_reader(&self.path, &self.asset, &self.track, seconds)?;
            self.reader = reader;
            self.output = output;
            Ok(())
        })
    }

    fn next_frame(&mut self) -> Result<Option<RawFrame>, MediaError> {
        loop {
            let result = autoreleasepool(|_| -> Result<Option<Option<RawFrame>>, MediaError> {
                // SAFETY: reading from a started reader we own.
                let Some(sample) = (unsafe { self.output.copyNextSampleBuffer() }) else {
                    // SAFETY: status/error getters.
                    return match unsafe { self.reader.status() } {
                        AVAssetReaderStatus::Failed => {
                            let reason = unsafe { self.reader.error() }.map_or_else(
                                || "decoding failed".to_owned(),
                                |e| e.localizedDescription().to_string(),
                            );
                            Err(error(&self.path, reason))
                        }
                        _ => Ok(Some(None)), // end of file
                    };
                };
                // SAFETY: sample buffer accessors on a valid sample.
                let time = seconds(unsafe { sample.presentation_time_stamp() });
                let Some(image) = (unsafe { sample.image_buffer() }) else {
                    return Ok(None); // a sample without pixels; try the next one
                };
                Ok(Some(Some(copy_bgra(&image, time))))
            })?;
            match result {
                Some(frame) => return Ok(frame),
                None => continue,
            }
        }
    }
}

impl Drop for AvfDecoder {
    fn drop(&mut self) {
        // SAFETY: cancelling a reader we own.
        unsafe { self.reader.cancelReading() };
    }
}

/// Copies a BGRA pixel buffer into tightly packed RGBA.
fn copy_bgra(image: &objc2_core_video::CVPixelBuffer, time: f64) -> RawFrame {
    // SAFETY: the buffer is locked read-only while its memory is accessed, and
    // rows are read within height × bytes-per-row.
    unsafe {
        CVPixelBufferLockBaseAddress(image, CVPixelBufferLockFlags::ReadOnly);
        let width = CVPixelBufferGetWidth(image);
        let height = CVPixelBufferGetHeight(image);
        let stride = CVPixelBufferGetBytesPerRow(image);
        let base = CVPixelBufferGetBaseAddress(image).cast::<u8>();
        let mut data = Vec::with_capacity(width * height * 4);
        if !base.is_null() {
            let src = std::slice::from_raw_parts(base, stride * height);
            for row in src.chunks_exact(stride) {
                for px in row[..width * 4].chunks_exact(4) {
                    data.extend_from_slice(&[px[2], px[1], px[0], 255]);
                }
            }
        }
        CVPixelBufferUnlockBaseAddress(image, CVPixelBufferLockFlags::ReadOnly);
        RawFrame {
            time,
            width: width as u32,
            height: height as u32,
            stride: (width * 4) as u32,
            data,
        }
    }
}
