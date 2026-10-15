use super::super::status_surface_preview::StatusSurfacePreviewData;
use super::super::status_surface_preview::StatusSurfacePreviewItem;
use super::*;
use pretty_assertions::assert_eq;
#[test]
fn cutex_rows_share_preview_boundaries_and_do_not_create_blank_rows() {
    use StatusLineItem::CurrentDir as B;
    use StatusLineItem::CutexNewline as N;
    use StatusLineItem::ModelName as A;
    assert_eq!(split([A, N, B, N, A]), [vec![A], vec![B, A]]);
    let data = StatusSurfacePreviewData::from_iter([
        (StatusSurfacePreviewItem::Model, "sol"),
        (StatusSurfacePreviewItem::CurrentDir, "~/中文"),
    ]);
    let text = |items: Vec<StatusLineItem>| {
        data.status_lines_for_items(items, false)
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
    };
    assert_eq!(text(vec![A, N, B]), vec!["sol", "~/中文"]);
    assert_eq!(text(vec![N, A, N]), vec!["sol"]);
    assert_eq!(text(vec![N]), Vec::<String>::new());
    assert_eq!(text(vec![A, N, A, B]), vec!["sol", "sol · ~/中文"]);
    let blank = StatusSurfacePreviewData::from_iter([(StatusSurfacePreviewItem::Model, "   ")]);
    assert_eq!(blank.status_lines_for_items([A, N], false).len(), 1);
}

#[test]
fn spacing_keeps_columns_without_dots_or_disturbing_normal_separators() {
    use StatusLineItem::CurrentDir as B;
    use StatusLineItem::CutexNewline as N;
    use StatusLineItem::CutexSpace4 as S;
    use StatusLineItem::ModelName as A;
    let data = StatusSurfacePreviewData::from_iter([
        (StatusSurfacePreviewItem::Model, "sol"),
        (StatusSurfacePreviewItem::CurrentDir, "~/中文"),
    ]);
    let rows = data.status_lines_for_items([A, B, N, S, A, S, B], false);
    assert_eq!(
        rows.iter().map(ToString::to_string).collect::<Vec<_>>(),
        vec!["sol · ~/中文", "    sol    ~/中文"]
    );
    assert_eq!(rows[1].width(), 17);
    // Invisible separators still occupy span slots used by notification styling and animation.
    assert_eq!(rows[1].spans.len(), 7);
}
