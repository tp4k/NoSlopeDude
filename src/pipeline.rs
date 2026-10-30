//! The stage sequence: target resolution, discovery, then the five scan
//! stages in order.

use crate::config;
use crate::discover::{self, DiscoverResult};
use crate::lower::{self, IrFile};
use crate::model::{ClonesResult, MetricsResult, ParseFailure, RulesResult, ScanSettings};
use crate::report::{Report, ReportInput};
use crate::target::{self, ResolvedTarget};
use crate::{clones, metrics, parse, report, rules};

/// Everything produced by one run of the pipeline.
pub struct PipelineOutput {
    pub settings: ScanSettings,
    pub resolved_target: ResolvedTarget,
    pub discover: DiscoverResult,
    pub parse_failures: Vec<ParseFailure>,
    pub metrics: MetricsResult,
    pub clones: ClonesResult,
    pub rules: RulesResult,
    pub report: Report,
    /// The IR built per successfully parsed file. WS-9 (C1): this is the one lowering `run` below
    /// performs -- `metrics`, `clones` and `rules` above are each computed
    /// from this same `Vec<IrFile>`, index-aligned with the pipeline's own
    /// `parsed_files`, instead of each lowering independently.
    pub ir: Vec<IrFile>,
}

/// Resolves `target_input`, walks it, and runs the five scan stages with
/// `settings` exactly as given.
pub fn run(target_input: &str, settings: ScanSettings) -> anyhow::Result<PipelineOutput> {
    run_with_threshold(target_input, settings, ThresholdSource::Settings)
}

/// Where a run's clone threshold comes from.
enum ThresholdSource {
    /// `ScanSettings::min_clone_lines`, as the caller set it.
    Settings,
    /// `measurement.min_clone_lines` of the resolved root's `nsd.yml`
    /// (amendment A2), else `ScanSettings::min_clone_lines`.
    RootConfig,
}

/// Like `run`, but `settings.min_clone_lines` is only the fallback: the
/// scanned root's `nsd.yml` decides it when it sets
/// `measurement.min_clone_lines`. An invalid `nsd.yml` is an error.
pub fn run_with_root_config(
    target_input: &str,
    settings: ScanSettings,
) -> anyhow::Result<PipelineOutput> {
    run_with_threshold(target_input, settings, ThresholdSource::RootConfig)
}

fn run_with_threshold(
    target_input: &str,
    mut settings: ScanSettings,
    source: ThresholdSource,
) -> anyhow::Result<PipelineOutput> {
    let target = target::classify(target_input);
    let resolved_target = target::resolve(&target)?;
    if let ThresholdSource::RootConfig = source {
        if let Some(config) = config::load_scan_root(&resolved_target.root)? {
            settings.min_clone_lines = config.measurement.min_clone_lines;
        }
    }
    let discover = discover::discover(&resolved_target.root, &settings)?;

    let (parsed_files, parse_failures) =
        parse::parse_all(&resolved_target.root, &discover.discovered);
    let (ir, unanalyzed_lines) = lower::lower_all_inventoried(&parsed_files);
    let metrics = metrics::run_with_ir(
        &parsed_files,
        &ir,
        &unanalyzed_lines,
        !parse_failures.is_empty(),
    );
    let clones = clones::run_with_ir(&parsed_files, &ir, settings.min_clone_lines);
    let rules = rules::run_with_ir(&parsed_files, &ir, &metrics, &clones);
    let report_input = ReportInput {
        target: &target,
        target_input,
        root: &resolved_target.root,
        revision: &resolved_target.revision,
        settings: &settings,
        discover: &discover,
        parse_failures: &parse_failures,
        metrics: &metrics,
        clones: &clones,
        rules: &rules,
    };
    let report = report::run(&report_input)?;

    Ok(PipelineOutput {
        settings,
        resolved_target,
        discover,
        parse_failures,
        metrics,
        clones,
        rules,
        report,
        ir,
    })
}
