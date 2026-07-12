// SPDX-License-Identifier: GPL-3.0

//! Local file converter/transcoder/ripper view — pick files (audio, video
//! containers, or `.cue` sheets), an output format/rate, and run the queue.

use std::path::Path;

use cosmic::iced::{Alignment, Length};
use cosmic::widget;

use crate::convert::{ConvertJob, JobKind, JobState, OutputFormat};
use crate::fl;
use crate::views::common;

/// Sample-rate dropdown options: `None` keeps the source rate.
pub const SAMPLE_RATE_OPTIONS: &[Option<u32>] = &[None, Some(44_100), Some(48_000), Some(96_000)];

/// Messages from the convert view.
#[derive(Debug, Clone)]
pub enum ConvertMessage {
    /// Open the (multi-select) file picker to add jobs.
    AddFiles,
    /// Open the directory picker to change the output directory.
    PickOutputDir,
    /// User picked an entry in the format dropdown.
    FormatSelected(usize),
    /// User picked an entry in the sample-rate dropdown.
    RateSelected(usize),
    /// Run every queued job.
    StartQueue,
    /// Cancel a specific job by id.
    CancelJob(u64),
    /// Drop every finished (done/failed/cancelled) job from the list.
    ClearFinished,
}

/// Localized label for a format dropdown entry.
fn format_label(format: OutputFormat) -> String {
    match format {
        OutputFormat::Flac => fl!("convert-format-flac"),
        OutputFormat::Wav16 => fl!("convert-format-wav16"),
        OutputFormat::Wav24 => fl!("convert-format-wav24"),
        OutputFormat::Wav32Float => fl!("convert-format-wav32float"),
    }
}

/// Localized label for a sample-rate dropdown entry.
fn rate_label(rate: Option<u32>) -> String {
    match rate {
        None => fl!("convert-rate-source"),
        Some(hz) => format!("{hz} Hz"),
    }
}

pub fn convert_view<'a>(
    jobs: &'a [ConvertJob],
    out_dir: &'a Path,
    format_index: usize,
    rate_index: usize,
) -> cosmic::Element<'a, ConvertMessage> {
    let mut col = widget::Column::new().spacing(12).padding(16);

    let dir_text = out_dir.display().to_string();
    let controls = widget::Column::new()
        .spacing(8)
        .push(
            widget::Row::new()
                .push(widget::button::suggested(fl!("convert-add-files")).on_press(ConvertMessage::AddFiles))
                .push(common::cell_text(fl!("convert-output-dir")))
                .push(common::clipped_cell(common::cell_text(dir_text).into()))
                .push(widget::button::standard(fl!("convert-choose-dir")).on_press(ConvertMessage::PickOutputDir))
                .spacing(8)
                .align_y(Alignment::Center),
        )
        .push(
            widget::Row::new()
                .push(common::cell_text(fl!("convert-format")))
                .push(widget::dropdown(
                    OutputFormat::ALL.iter().map(|&f| format_label(f)).collect::<Vec<_>>(),
                    Some(format_index),
                    ConvertMessage::FormatSelected,
                ))
                .push(common::cell_text(fl!("convert-sample-rate")))
                .push(widget::dropdown(
                    SAMPLE_RATE_OPTIONS.iter().map(|&r| rate_label(r)).collect::<Vec<_>>(),
                    Some(rate_index),
                    ConvertMessage::RateSelected,
                ))
                .spacing(8)
                .align_y(Alignment::Center),
        )
        .push(
            widget::Row::new()
                .push(
                    widget::button::suggested(fl!("convert-start"))
                        .on_press_maybe(has_queued(jobs).then_some(ConvertMessage::StartQueue)),
                )
                .push(
                    widget::button::standard(fl!("convert-clear-finished"))
                        .on_press_maybe(has_finished(jobs).then_some(ConvertMessage::ClearFinished)),
                )
                .spacing(8),
        );

    col = col.push(controls);
    col = col.push(widget::divider::horizontal::default());

    if jobs.is_empty() {
        col = col.push(common::empty_state(
            "media-import-audio-symbolic",
            fl!("no-convert-jobs"),
            fl!("convert-empty-hint"),
        ));
        return col.into();
    }

    let mut list = widget::Column::new().spacing(2);
    for job in jobs {
        list = list.push(job_row(job));
    }

    col = col.push(widget::scrollable(widget::container(list).width(Length::Fill)).height(Length::Fill));
    col.into()
}

fn has_queued(jobs: &[ConvertJob]) -> bool {
    jobs.iter().any(|j| j.state == JobState::Queued)
}

fn has_finished(jobs: &[ConvertJob]) -> bool {
    jobs.iter()
        .any(|j| matches!(j.state, JobState::Done | JobState::Failed(_) | JobState::Cancelled))
}

fn kind_label(kind: JobKind) -> String {
    match kind {
        JobKind::Convert => fl!("convert-kind-convert"),
        JobKind::CueSplit => fl!("convert-kind-cuesplit"),
    }
}

fn state_label(state: &JobState) -> String {
    match state {
        JobState::Queued => fl!("convert-state-queued"),
        JobState::Running => fl!("convert-state-running"),
        JobState::Done => fl!("convert-state-done"),
        JobState::Failed(error) => fl!("convert-state-failed", error = error.clone()),
        JobState::Cancelled => fl!("convert-state-cancelled"),
    }
}

fn job_row(job: &ConvertJob) -> cosmic::Element<'_, ConvertMessage> {
    let filename = job.source.file_name().and_then(|n| n.to_str()).unwrap_or("?");

    let mut info = widget::Column::new()
        .push(common::cell_text(filename))
        .push(common::cell_caption(format!("{} — {}", kind_label(job.kind), state_label(&job.state))))
        .spacing(2);

    if job.state == JobState::Running {
        info = info.push(
            widget::progress_bar::determinate_linear(job.progress_permille() as f32 / 1000.0)
                .width(Length::Fill),
        );
    }

    let cancellable = matches!(job.state, JobState::Queued | JobState::Running);
    let cancel_btn = widget::tooltip(
        widget::button::icon(widget::icon::from_name("process-stop-symbolic").size(16))
            .class(cosmic::theme::Button::Destructive)
            .on_press_maybe(cancellable.then_some(ConvertMessage::CancelJob(job.id))),
        widget::text::caption(fl!("convert-cancel-tooltip")),
        widget::tooltip::Position::Top,
    );

    widget::container(
        widget::Row::new()
            .push(widget::icon::from_name("audio-x-generic-symbolic").size(32))
            .push(common::clipped_cell(info.into()))
            .push(cancel_btn)
            .spacing(12)
            .align_y(Alignment::Center)
            .padding(8),
    )
    .width(Length::Fill)
    .into()
}
