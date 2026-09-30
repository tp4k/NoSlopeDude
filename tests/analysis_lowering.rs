use std::path::Path;

use nsd::analysis::analyze_file;
use nsd::lower;

#[test]
fn test_one_analysis_lowers_once() {
    let source = b"class C {\n  void m() {\n    try { f(); } catch (Exception e) { }\n  }\n}\n";
    let before = lower::lowering_count();
    let analysis = analyze_file(Path::new("C.java"), source).expect("analyzable");
    let after = lower::lowering_count();
    assert_eq!(analysis.callables.len(), 1);
    assert_eq!(analysis.findings.len(), 1);
    assert_eq!(after - before, 1);
}
