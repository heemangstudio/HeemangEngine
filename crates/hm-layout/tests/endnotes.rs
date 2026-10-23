//! Endnote composition measured against Mac Hancom exports.

mod common;

use hm_layout::{FrameKind, ListRef};

/// A picture crossing the column foot flows on independently. Its anchor
/// and the following equation remain in the first column, while the next
/// note follows the picture in the second column.
#[test]
fn endnote_drawings_carry_without_moving_their_anchor_text() {
    let Some((corpus, fonts)) = common::setup() else {
        return;
    };
    for ext in ["hwp", "hwpx"] {
        let path = corpus.join(format!(
            "3-11월_실전_통합_2024-구분선위20미주사이0구분선아래20.{ext}"
        ));
        let Some(doc) = common::read(&path) else {
            continue;
        };
        let layout = hm_layout::layout(&doc, &fonts);
        assert_eq!(layout.pages.len(), 21, "{ext}");
        let page = &layout.pages[10];
        let note = ListRef::Control {
            section: 0,
            para: 99,
            control: 0,
        };
        let anchor = page
            .frames
            .iter()
            .find(|f| f.kind == FrameKind::Endnotes && f.list == note && !f.lines.is_empty())
            .expect("anchor text stays on page 11");
        assert!(anchor.lines.iter().any(|l| l.para == 12), "{ext}");
        assert!(anchor.objects.is_empty(), "{ext}");
        let picture = page
            .frames
            .iter()
            .find(|f| f.kind == FrameKind::Endnotes && f.list == note && !f.objects.is_empty())
            .expect("picture continues in the next column");
        assert!(picture.rect.x > anchor.rect.x, "{ext}");
        assert!((picture.objects[0].rect.y - 6803).abs() <= 12, "{ext}");
        let next = page
            .frames
            .iter()
            .find(|f| {
                f.list
                    == ListRef::Control {
                        section: 0,
                        para: 108,
                        control: 0,
                    }
            })
            .expect("note 13 follows the picture");
        assert_eq!(next.rect.x, picture.rect.x, "{ext}");
        assert!(
            next.rect.y >= picture.objects[0].rect.y + picture.objects[0].rect.h,
            "{ext}"
        );
        // Hancom's note-13 heading baseline is 19272 HU, with a 765 HU
        // text baseline in the heading's top-aligned 900 HU line.
        assert!(
            (next.rect.y + 765 - 19272).abs() <= 12,
            "{ext}: {}",
            next.rect.y
        );
    }
}
