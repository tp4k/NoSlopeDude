//! The stage sequence: target resolution, discovery, then the five scan
//! stages in order.

use crate::discover::{self, DiscoverResult};
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
}

/// Resolves `target_input`, walks it, and runs the five scan stages.
pub fn run(target_input: &str, settings: ScanSettings) -> anyhow::Result<PipelineOutput> {
    let target = target::classify(target_input);
    let resolved_target = target::resolve(&target)?;
    let discover = discover::discover(&resolved_target.root, &settings)?;

    let (parsed_files, parse_failures) =
        parse::parse_all(&resolved_target.root, &discover.discovered);
    let metrics = metrics::run(&parsed_files, !parse_failures.is_empty());
    let clones = clones::run(&parsed_files, settings.min_clone_lines);
    let rules = rules::run(&parsed_files, &metrics, &clones);
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
    })
}
