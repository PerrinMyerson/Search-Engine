use super::*;

#[test]
fn renders_ordered_and_unordered_list_markers() {
    let render = render_html(
        "mem://lists",
        br#"
            <html><body>
              <ol start="3">
                <li>Install Rust</li>
                <li value="7">Run tests</li>
                <li>Ship browser</li>
              </ol>
              <ol reversed>
                <li>Third</li>
                <li>Second</li>
              </ol>
              <ul><li>Plain bullet</li></ul>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "3. Install Rust\n7. Run tests\n8. Ship browser\n2. Third\n1. Second\n- Plain bullet"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "3. Install Rust".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "7. Run tests".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "8. Ship browser".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "2. Third".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "1. Second".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 5,
                text: "- Plain bullet".to_owned(),
            },
        ]
    );
}

#[test]
fn ordered_list_counters_skip_hidden_items() {
    let render = render_html(
        "mem://hidden-list-items",
        br#"
            <html><body>
              <ol start="4">
                <li hidden value="20">Hidden reset</li>
                <li>Visible start</li>
                <li>Visible next</li>
              </ol>
              <ol reversed>
                <li>Two</li>
                <li hidden>Hidden middle</li>
                <li>One</li>
              </ol>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "4. Visible start\n5. Visible next\n2. Two\n1. One"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "4. Visible start".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "5. Visible next".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "2. Two".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "1. One".to_owned(),
            },
        ]
    );
}

#[test]
fn renders_ordered_list_type_markers() {
    let render = render_html(
        "mem://typed-lists",
        br#"
            <html><body>
              <ol type="a" start="26">
                <li>Twenty six</li>
                <li>Twenty seven</li>
                <li value="28">Twenty eight</li>
              </ol>
              <ol type="I" start="4">
                <li>Four</li>
                <li>Nine</li>
              </ol>
              <ol type="i" reversed start="3">
                <li>Three</li>
                <li>Two</li>
              </ol>
              <ol type="A">
                <li>Upper alpha</li>
              </ol>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "z. Twenty six\naa. Twenty seven\nab. Twenty eight\nIV. Four\nV. Nine\niii. Three\nii. Two\nA. Upper alpha"
    );
    assert_eq!(
        render
            .display_list
            .iter()
            .filter_map(|command| match command {
                DisplayCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            "z. Twenty six",
            "aa. Twenty seven",
            "ab. Twenty eight",
            "IV. Four",
            "V. Nine",
            "iii. Three",
            "ii. Two",
            "A. Upper alpha",
        ]
    );
}

#[test]
fn ordered_list_item_type_overrides_parent_marker_style() {
    let render = render_html(
        "mem://list-item-type",
        br#"
            <html><body>
              <ol type="a" start="2">
                <li>Lower alpha</li>
                <li type="I">Upper roman</li>
                <li type="1" value="9">Decimal override</li>
                <li type="A">Upper alpha</li>
                <li>Back to lower alpha</li>
              </ol>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "b. Lower alpha\nIII. Upper roman\n9. Decimal override\nJ. Upper alpha\nk. Back to lower alpha"
    );
    assert_eq!(
        render
            .display_list
            .iter()
            .filter_map(|command| match command {
                DisplayCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            "b. Lower alpha",
            "III. Upper roman",
            "9. Decimal override",
            "J. Upper alpha",
            "k. Back to lower alpha",
        ]
    );
}

#[test]
fn list_marker_attributes_ignore_surrounding_whitespace() {
    let render = render_html(
        "mem://list-marker-attribute-normalization",
        br#"
            <html><body>
              <ol type=" A " start=" +26 ">
                <li>Upper alpha</li>
                <li type=" i " value=" 9 ">Lower roman</li>
              </ol>
              <ul type=" CIRCLE ">
                <li>Uppercase circle</li>
                <li type=" SQUARE ">Uppercase square</li>
              </ul>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Z. Upper alpha\nix. Lower roman\no Uppercase circle\n* Uppercase square"
    );
    assert_eq!(
        render
            .display_list
            .iter()
            .filter_map(|command| match command {
                DisplayCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            "Z. Upper alpha",
            "ix. Lower roman",
            "o Uppercase circle",
            "* Uppercase square",
        ]
    );
}

#[test]
fn css_list_style_type_controls_markers() {
    let render = render_html(
        "mem://css-list-style-type",
        br#"
            <html><head><style>
              ol.alpha { list-style-type: upper-alpha }
              ul.square { list-style: square inside }
              ul.none { list-style-type: none }
              li.roman { list-style-type: lower-roman }
              li.hidden { display: none }
            </style></head><body>
              <ol class="alpha" start="2">
                <li>Beta</li>
                <li class="roman" value="4">Roman item</li>
                <li>Delta</li>
              </ol>
              <ul class="square">
                <li>Square item</li>
                <li style="list-style-type: circle">Circle item</li>
              </ul>
              <ul class="none">
                <li>No marker</li>
              </ul>
              <ol>
                <li>One</li>
                <li class="hidden" value="9">Hidden reset</li>
                <li>Two</li>
              </ol>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "B. Beta\niv. Roman item\nE. Delta\n* Square item\no Circle item\nNo marker\n1. One\n2. Two"
    );
    assert_eq!(
        render
            .display_list
            .iter()
            .filter_map(|command| match command {
                DisplayCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            "B. Beta",
            "iv. Roman item",
            "E. Delta",
            "* Square item",
            "o Circle item",
            "No marker",
            "1. One",
            "2. Two",
        ]
    );
}

#[test]
fn css_display_list_item_controls_marker_generation() {
    let render = render_html(
        "mem://css-display-list-item",
        br#"
            <html><body>
              <ul>
                <li style="display: inline">Inline item</li>
                <li>Default item</li>
              </ul>
              <ol>
                <li>One</li>
                <li style="display: block" value="9">Block item</li>
                <li>Two</li>
              </ol>
              <div style="display: list-item; list-style-type: square">Generated marker</div>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Inline item\n- Default item\n1. One\nBlock item\n2. Two\n* Generated marker"
    );
    assert_eq!(
        render
            .display_list
            .iter()
            .filter_map(|command| match command {
                DisplayCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            "Inline item",
            "- Default item",
            "1. One",
            "Block item",
            "2. Two",
            "* Generated marker",
        ]
    );
    assert_eq!(
        render
            .layout_boxes
            .iter()
            .filter(|layout_box| matches!(layout_box.tag.as_str(), "li" | "div"))
            .map(|layout_box| (layout_box.tag.as_str(), layout_box.kind.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("li", "inline"),
            ("li", "list-item"),
            ("li", "list-item"),
            ("li", "block"),
            ("li", "list-item"),
            ("div", "list-item"),
        ]
    );
}

#[test]
fn indents_nested_list_markers() {
    let render = render_html(
        "mem://nested-lists",
        br#"
            <html><body>
              <ul>
                <li>Parent
                  <ul>
                    <li>Child
                      <ol start="2">
                        <li>Grandchild</li>
                      </ol>
                    </li>
                  </ul>
                </li>
                <li>Sibling</li>
              </ul>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "- Parent\no Child\n2. Grandchild\n- Sibling");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "- Parent".to_owned(),
            },
            DisplayCommand::Text {
                x: 2,
                y: 1,
                text: "o Child".to_owned(),
            },
            DisplayCommand::Text {
                x: 4,
                y: 2,
                text: "2. Grandchild".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "- Sibling".to_owned(),
            },
        ]
    );
}

#[test]
fn unordered_list_markers_use_all_list_ancestor_depth() {
    let render = render_html(
        "mem://mixed-list-depth",
        br#"
            <html><body>
              <ol>
                <li>Ordered parent
                  <ul>
                    <li>Unordered child
                      <ol>
                        <li>Ordered grandchild
                          <ul>
                            <li>Deep unordered</li>
                          </ul>
                        </li>
                      </ol>
                    </li>
                  </ul>
                </li>
              </ol>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "1. Ordered parent\no Unordered child\n1. Ordered grandchild\n* Deep unordered"
    );
    assert_eq!(
        render
            .display_list
            .iter()
            .filter_map(|command| match command {
                DisplayCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            "1. Ordered parent",
            "o Unordered child",
            "1. Ordered grandchild",
            "* Deep unordered",
        ]
    );
}

#[test]
fn renders_unordered_list_type_markers() {
    let render = render_html(
        "mem://unordered-types",
        br#"
            <html><body>
              <ul type="circle">
                <li>Circle parent</li>
                <li type="square">Square item</li>
                <li type="disc">Disc item</li>
                <li>Circle again</li>
              </ul>
              <ul type="square">
                <li>Square parent</li>
              </ul>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "o Circle parent\n* Square item\n- Disc item\no Circle again\n* Square parent"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "o Circle parent".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "* Square item".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "- Disc item".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "o Circle again".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "* Square parent".to_owned(),
            },
        ]
    );
}

#[test]
fn preserves_preformatted_text_spaces_and_blank_lines() {
    let render = render_html(
        "mem://preformatted",
        br#"<html><body><pre>fn main() {
  println!("hi");

done</pre><p style="white-space: pre">A  B
C</p></body></html>"#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "fn main() {\n  println!(\"hi\");\n\ndone\nA  B\nC"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "fn main() {".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "  println!(\"hi\");".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "done".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "A  B".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 5,
                text: "C".to_owned(),
            },
        ]
    );
}

#[test]
fn css_white_space_nowrap_suppresses_soft_wrapping() {
    let render = render_html(
        "mem://nowrap",
        br#"
            <html><body>
              <p>Alpha Beta Gamma Delta</p>
              <p style="white-space: nowrap">Alpha Beta Gamma Delta</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Alpha Beta Gamma\nDelta\nAlpha Beta Gamma Delta"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Alpha Beta Gamma".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "Delta".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Alpha Beta Gamma Delta".to_owned(),
            },
        ]
    );
}

#[test]
fn inline_text_spacing_preserves_punctuation_boundaries() {
    let render = render_html(
        "mem://inline-punctuation-spacing",
        br##"
            <html><body>
              <p>The <a href="/war">Baptist War</a>, broke out.[<a href="#cite">59</a>]</p>
              <p><span>Alpha</span><span>Beta</span><span> Gamma</span><span> Delta</span></p>
              <p><span>[</span><a href="#ref">12</a><span>]</span> marker</p>
            </body></html>
            "##,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "The Baptist War, broke out.[59]\nAlphaBeta Gamma Delta\n[12] marker"
    );
}

#[test]
fn css_white_space_pre_line_preserves_newlines_and_wraps() {
    let render = render_html(
        "mem://pre-line",
        br#"
            <html><body>
              <p style="white-space: pre-line">Alpha   Beta
Gamma Delta Epsilon Zeta</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Alpha Beta\nGamma Delta Epsilon\nZeta");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Alpha Beta".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "Gamma Delta Epsilon".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Zeta".to_owned(),
            },
        ]
    );
}

#[test]
fn css_white_space_pre_wrap_preserves_spaces_and_soft_wraps() {
    let render = render_html(
        "mem://pre-wrap",
        br#"
            <html><body>
              <p style="white-space: pre-wrap">A  B
ABCDEFGHIJKLMNOPQRSTUV</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "A  B\nABCDEFGHIJKLMNOPQRST\nUV");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "A  B".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "ABCDEFGHIJKLMNOPQRST".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "UV".to_owned(),
            },
        ]
    );
}

#[test]
fn css_white_space_break_spaces_preserves_spaces_and_soft_wraps() {
    let render = render_html(
        "mem://break-spaces",
        br#"
            <html><body>
              <p style="white-space: break-spaces">A    B
C</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 3,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "A  \n  B\nC");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "A  ".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "  B".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "C".to_owned(),
            },
        ]
    );
}

#[test]
fn wbr_creates_zero_width_soft_break_opportunity() {
    let render = render_html(
        "mem://wbr",
        br#"
            <html><body>
              <p>Alpha<wbr>Beta Gamma</p>
              <p>Fit<wbr>Here</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 9,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "AlphaBeta\nGamma\nFitHere");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "AlphaBeta".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "Gamma".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "FitHere".to_owned(),
            },
        ]
    );
}

#[test]
fn css_line_height_adds_visual_row_spacing() {
    let render = render_html(
        "mem://line-height",
        br#"
            <html><body>
              <p style="line-height: 24px">Tall line</p>
              <p>Normal line</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Tall line\n\nNormal line");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Tall line".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Normal line".to_owned(),
            },
        ]
    );
}

#[test]
fn css_text_indent_offsets_first_line_and_affects_wrap_width() {
    let render = render_html(
        "mem://text-indent",
        br#"
            <html><body>
              <p style="text-indent: 16px">Alpha Beta Gamma Delta</p>
              <p>Plain text</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "  Alpha Beta Gamma\nDelta\nPlain text");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "  Alpha Beta Gamma".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "Delta".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Plain text".to_owned(),
            },
        ]
    );
}

#[test]
fn css_relative_length_units_affect_layout_text_metrics() {
    let render = render_html(
        "mem://relative-length-units",
        br#"
            <html><body>
              <div style="width:4rem; padding-left:1em; margin-left:1rem">Alpha Beta</div>
              <p style="text-indent:2em">Indented words</p>
              <p style="word-spacing:1ch">Wide gap</p>
              <p style="letter-spacing:0.5rem">AB</p>
              <p style="line-height:1rem">Tall</p>
              <p>After</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 24,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Alpha\nBeta\n    Indented words\nWide  gap\nA B\nTall\n\nAfter"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 4,
                y: 0,
                text: "Alpha".to_owned(),
            },
            DisplayCommand::Text {
                x: 4,
                y: 1,
                text: "Beta".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "    Indented words".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "Wide  gap".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "A B".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 5,
                text: "Tall".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 7,
                text: "After".to_owned(),
            },
        ]
    );
}

#[test]
fn css_overflow_wrap_break_word_breaks_long_words() {
    let render = render_html(
        "mem://overflow-wrap",
        br#"
            <html><body>
              <p style="overflow-wrap: break-word">ABCDEFGHIJK</p>
              <p>ABCDEFGHIJK</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 8,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "ABCDEFGH\nIJK\nABCDEFGHIJK");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "ABCDEFGH".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "IJK".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "ABCDEFGHIJK".to_owned(),
            },
        ]
    );
}

#[test]
fn css_word_break_break_all_breaks_long_words() {
    let render = render_html(
        "mem://word-break",
        br#"
            <html><body>
              <p style="word-break: break-all">One AlphaBetaGamma</p>
              <p style="word-break: normal">AlphaBetaGamma</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 10,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "One AlphaB\netaGamma\nAlphaBetaGamma");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "One AlphaB".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "etaGamma".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "AlphaBetaGamma".to_owned(),
            },
        ]
    );
}

#[test]
fn css_text_transform_changes_rendered_text() {
    let render = render_html(
        "mem://text-transform",
        br#"
            <html><head><style>
              .shout { text-transform: uppercase }
              .quiet { text-transform: lowercase }
              .plain { text-transform: none }
            </style></head><body>
              <p class="shout">Rust browser <span class="quiet">ENGINE</span> <span class="plain">MiXeD</span></p>
              <p style="text-transform: lowercase">LOUD Text</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "RUST BROWSER engine MiXeD\nloud text");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "RUST BROWSER engine MiXeD".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "loud text".to_owned(),
            },
        ]
    );
}

#[test]
fn css_text_transform_capitalize_uses_word_boundaries() {
    let render = render_html(
        "mem://text-transform-capitalize",
        br#"
            <html><head><style>
              .title { text-transform: capitalize }
              .plain { text-transform: none }
            </style></head><body>
              <p class="title">rust-browser layout ENGINE <span class="plain">raw mix</span> again</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Rust-Browser Layout ENGINE raw mix Again");
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Text {
            x: 0,
            y: 0,
            text: "Rust-Browser Layout ENGINE raw mix Again".to_owned(),
        }]
    );
}

#[test]
fn css_letter_spacing_expands_text_runs_and_wrap_width() {
    let render = render_html(
        "mem://letter-spacing",
        br#"
            <html><body>
              <p style="letter-spacing: 8px">AB CD EF</p>
              <p style="letter-spacing: 8px">Wide <span style="letter-spacing: normal">gap ok</span> end</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 16,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "A B C D E F\nW i d e gap ok\ne n d");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "A B C D E F".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "W i d e gap ok".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "e n d".to_owned(),
            },
        ]
    );
}

#[test]
fn css_letter_spacing_applies_inside_pre_wrap_segments() {
    let render = render_html(
        "mem://letter-spacing-pre-wrap",
        br#"
            <html><body>
              <p style="white-space: pre-wrap; letter-spacing: 8px">ABCD</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 3,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "A B\nC D");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "A B".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "C D".to_owned(),
            },
        ]
    );
}

#[test]
fn css_word_spacing_expands_inter_word_gaps_and_wrap_width() {
    let render = render_html(
        "mem://word-spacing",
        br#"
            <html><body>
              <p style="word-spacing: 16px">Alpha Beta Gamma Delta</p>
              <p style="word-spacing: 16px">Wide <span style="word-spacing: normal">normal gap</span> done</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Alpha   Beta   Gamma\nDelta\nWide   normal gap\ndone"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Alpha   Beta   Gamma".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "Delta".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Wide   normal gap".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "done".to_owned(),
            },
        ]
    );
}

#[test]
fn flows_simple_table_cells_across_rows() {
    let render = render_html(
        "mem://table",
        br#"
            <html><body>
              <table>
                <thead>
                  <tr><th>Name</th><th>Status</th></tr>
                </thead>
                <tbody>
                  <tr><td>Parser</td><td>running</td></tr>
                  <tr><td>Layout</td><td>next</td></tr>
                </tbody>
              </table>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Name    Status\nParser  running\nLayout  next");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Name    Status".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "Parser  running".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Layout  next".to_owned(),
            },
        ]
    );
    assert!(hit_test_target_node(&render, 0, 1).is_some());
    assert_eq!(hit_test_target_node(&render, 6, 1), None);
    assert!(hit_test_target_node(&render, 8, 1).is_some());
}

#[test]
fn css_table_display_values_use_table_flow() {
    let render = render_html(
        "mem://css-table-display",
        br#"
            <html><head>
              <style>
                .table { display: table }
                .row { display: table-row }
                .cell { display: table-cell }
              </style>
            </head><body>
              <div class="table">
                <div class="row"><span class="cell">Name</span><span class="cell">Status</span></div>
                <div class="row"><span class="cell">Parser</span><span class="cell">running</span></div>
              </div>
              <p>After</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Name    Status\nParser  running\nAfter");
    assert_eq!(
        render
            .layout_boxes
            .iter()
            .filter(|layout_box| layout_box.kind == "table-cell")
            .map(|layout_box| (layout_box.tag.as_str(), layout_box.kind.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("span", "table-cell"),
            ("span", "table-cell"),
            ("span", "table-cell"),
            ("span", "table-cell"),
        ]
    );
    assert!(hit_test_target_node(&render, 0, 1).is_some());
    assert_eq!(hit_test_target_node(&render, 6, 1), None);
    assert!(hit_test_target_node(&render, 8, 1).is_some());
}

#[test]
fn table_colspan_spans_multiple_columns() {
    let render = render_html(
        "mem://table-colspan",
        br#"
            <html><body>
              <table>
                <tr><th colspan="2">Product</th><th>Price</th></tr>
                <tr><td>Browser</td><td>Engine</td><td>Fast</td></tr>
                <tr><td colspan="2">Total cost</td><td>Zero</td></tr>
              </table>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Product          Price\nBrowser  Engine  Fast\nTotal cost       Zero"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Product          Price".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "Browser  Engine  Fast".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Total cost       Zero".to_owned(),
            },
        ]
    );
    assert!(hit_test_target_node(&render, 1, 0).is_some());
    assert_eq!(hit_test_target_node(&render, 9, 0), None);
    assert!(hit_test_target_node(&render, 17, 0).is_some());
}

#[test]
fn table_rowspan_offsets_following_rows() {
    let render = render_html(
        "mem://table-rowspan",
        br#"
            <html><body>
              <table>
                <tr><th rowspan="2">Area</th><th>Metric</th></tr>
                <tr><td>Layout</td></tr>
                <tr><td>Speed</td><td>Fast</td></tr>
              </table>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Area   Metric\n       Layout\nSpeed  Fast");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Area   Metric".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "       Layout".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Speed  Fast".to_owned(),
            },
        ]
    );
    assert!(hit_test_target_node(&render, 1, 0).is_some());
    assert_eq!(hit_test_target_node(&render, 1, 1), None);
    assert!(hit_test_target_node(&render, 8, 1).is_some());
}

#[test]
fn table_cell_widths_set_column_minimums() {
    let render = render_html(
        "mem://table-cell-widths",
        br#"
            <html><body>
              <table>
                <tr><th style="width:80px">Name</th><th width="64">Status</th><th>Phase</th></tr>
                <tr><td>A</td><td>Ok</td><td>Go</td></tr>
              </table>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Name        Status   Phase\nA           Ok       Go"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Name        Status   Phase".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "A           Ok       Go".to_owned(),
            },
        ]
    );
    assert!(hit_test_target_node(&render, 0, 1).is_some());
    assert_eq!(hit_test_target_node(&render, 6, 1), None);
    assert!(hit_test_target_node(&render, 12, 1).is_some());
    assert!(hit_test_target_node(&render, 21, 1).is_some());
}

#[test]
fn table_colgroup_widths_set_column_minimums() {
    let render = render_html(
        "mem://table-colgroup-widths",
        br#"
            <html><body>
              <table>
                <colgroup>
                  <col span="2" style="width:64px">
                  <col width="80">
                  <col>
                </colgroup>
                <tr><th>A</th><th>B</th><th>C</th><th>D</th></tr>
                <tr><td>One</td><td>Two</td><td>Three</td><td>Four</td></tr>
              </table>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "A         B        C          D\nOne       Two      Three      Four"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "A         B        C          D".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "One       Two      Three      Four".to_owned(),
            },
        ]
    );
    assert!(hit_test_target_node(&render, 0, 1).is_some());
    assert_eq!(hit_test_target_node(&render, 5, 1), None);
    assert!(hit_test_target_node(&render, 10, 1).is_some());
    assert!(hit_test_target_node(&render, 19, 1).is_some());
    assert!(hit_test_target_node(&render, 30, 1).is_some());
}

#[test]
fn table_row_collection_ignores_nested_tables() {
    let parsed = parse_html(
        br#"
            <html><body>
              <table id="outer">
                <tr>
                  <td>
                    Outer
                    <table id="inner">
                      <tr><td>Nested</td><td>Row</td></tr>
                    </table>
                  </td>
                  <td>Side</td>
                </tr>
                <tr><td>Bottom</td><td>Cell</td></tr>
              </table>
            </body></html>
            "#,
    );
    let css_cascade = parse_css("");
    let outer_table = find_first_matching_selector(&parsed.dom, "table#outer").unwrap();
    let rows = table_rows(&parsed.dom, outer_table, &css_cascade);

    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter()
            .map(|&row| table_row_cell_count(&parsed.dom, row, &css_cascade))
            .collect::<Vec<_>>(),
        vec![2, 2]
    );
}

#[test]
fn indents_blockquotes_and_definition_descriptions_by_default() {
    let render = render_html(
        "mem://prose-indent",
        br#"
            <html><body>
              <blockquote><p>Quoted document text</p></blockquote>
              <dl>
                <dt>Engine</dt>
                <dd>Rust layout pipeline</dd>
              </dl>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Quoted document text\nEngine\nRust layout pipeline"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 4,
                y: 0,
                text: "Quoted document text".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "Engine".to_owned(),
            },
            DisplayCommand::Text {
                x: 4,
                y: 2,
                text: "Rust layout pipeline".to_owned(),
            },
        ]
    );
}

#[test]
fn links_have_default_text_shade_with_css_override() {
    let render = render_html(
        "mem://default-link-text-shade",
        br#"
            <html><head><style>
              a.override { color: red }
            </style></head><body>
              <p><a href="/profile">Profile <span>details</span></a></p>
              <p><a name="anchor">Anchor only</a></p>
              <p><a class="override" href="/alert">Alert</a></p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Profile details\nAnchor only\nAlert");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::StyledText {
                x: 0,
                y: 0,
                text: "Profile details".to_owned(),
                shade: 28,
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "Anchor only".to_owned(),
            },
            DisplayCommand::StyledText {
                x: 0,
                y: 2,
                text: "Alert".to_owned(),
                shade: 76,
            },
        ]
    );
}

#[test]
fn search_and_hgroup_use_block_flow_by_default() {
    let render = render_html(
        "mem://semantic-block-defaults",
        br#"
            <html><body>
              <span>Before</span>
              <search><label>Find</label> <input value="rust"></search>
              <hgroup><h1>Title</h1><p>Subtitle</p></hgroup>
              <span>After</span>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Before\nFind [rust]\nTitle\nSubtitle\nAfter");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Before".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "Find [rust]".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Title".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "Subtitle".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "After".to_owned(),
            },
        ]
    );
}

#[test]
fn css_visibility_hidden_reserves_layout_without_painting() {
    let render = render_html(
        "mem://visibility-hidden",
        br#"
            <html><body>
              <p>Before <span style="visibility:hidden">Hidden</span> After</p>
              <p style="visibility:hidden">Ghost <span style="visibility:visible">Shown</span> Gone</p>
              <hr style="visibility:hidden">
              <p>Tail</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "Before        After
      Shown     
Tail"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "Before".to_owned(),
            },
            DisplayCommand::Text {
                x: 13,
                y: 0,
                text: " After".to_owned(),
            },
            DisplayCommand::Text {
                x: 5,
                y: 1,
                text: " Shown".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "Tail".to_owned(),
            },
        ]
    );
}

#[test]
fn css_height_controls_block_and_image_extent() {
    let render = render_html(
        "mem://css-height",
        br#"
            <html><body>
              <img alt="plot" width="80" height="48" style="width:24px; height:24px">
              <div style="height:24px"></div>
              <p>After</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "After");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 3,
                height: 2,
                shade: 220,
                alt: Some("plot".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "After".to_owned(),
            },
        ]
    );
}

#[test]
fn css_image_single_axis_size_preserves_intrinsic_aspect_ratio() {
    let render = render_html(
        "mem://css-image-aspect-ratio",
        br#"
            <html><body>
              <img alt="wide" width="80" height="48" style="width:24px">
              <img alt="tall" width="80" height="48" style="height:24px">
              <p>After</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "After");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 3,
                height: 2,
                shade: 220,
                alt: Some("wide".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Image {
                x: 0,
                y: 2,
                width: 5,
                height: 2,
                shade: 220,
                alt: Some("tall".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "After".to_owned(),
            },
        ]
    );
}

#[test]
fn css_max_height_limits_image_placeholder_extent() {
    let render = render_html(
        "mem://css-max-height-image",
        br#"
            <html><body>
              <img alt="wide" width="80" height="48" style="max-height:24px">
              <img alt="floor" width="80" height="12" style="max-height:24px; min-height:36px">
              <p>After</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "After");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 10,
                height: 2,
                shade: 220,
                alt: Some("wide".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Image {
                x: 0,
                y: 2,
                width: 10,
                height: 3,
                shade: 220,
                alt: Some("floor".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 0,
                y: 5,
                text: "After".to_owned(),
            },
        ]
    );
}

#[test]
fn unresolved_image_placeholders_are_clamped_to_keep_text_visible() {
    let render = render_html(
        "mem://unresolved-image-placeholder-clamp",
        br#"
            <html><body>
              <img alt="hero" src="https://example.invalid/hero.jpg" width="800" height="480">
              <p>Contact details</p>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 60,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "Contact details");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Image {
                x: 0,
                y: 0,
                width: 60,
                height: 8,
                shade: 220,
                alt: Some("hero".to_owned()),
                url: None,
                decoded_width: None,
                decoded_height: None,
                decoded_hash: None,
            },
            DisplayCommand::Text {
                x: 0,
                y: 8,
                text: "Contact details".to_owned(),
            },
        ]
    );
}

#[test]
fn css_box_sizing_content_box_keeps_width_as_content_width() {
    let render = render_html(
        "mem://box-sizing",
        br#"
            <html><body>
              <div style="width:80px; padding-left:16px; padding-right:16px">AB CD EF</div>
              <div style="box-sizing:border-box; width:80px; padding-left:16px; padding-right:16px">AB CD EF</div>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 20,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(render.text, "AB CD EF\nAB CD\nEF");
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 2,
                y: 0,
                text: "AB CD EF".to_owned(),
            },
            DisplayCommand::Text {
                x: 2,
                y: 1,
                text: "AB CD".to_owned(),
            },
            DisplayCommand::Text {
                x: 2,
                y: 2,
                text: "EF".to_owned(),
            },
        ]
    );
}

#[test]
fn details_summary_hides_closed_content_and_marks_state() {
    let render = render_html(
        "mem://details-summary",
        br#"
            <html><body>
              <details>
                <summary>Closed title</summary>
                <p>Hidden body</p>
              </details>
              <details open>
                <summary>Open title</summary>
                <p>Visible body</p>
              </details>
              <details><p>No summary hidden</p></details>
              <details open><p>Fallback body</p></details>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 80,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "> Closed title
v Open title
Visible body
> Details
v Details
Fallback body"
    );
    assert_eq!(
        render.display_list,
        vec![
            DisplayCommand::Text {
                x: 0,
                y: 0,
                text: "> Closed title".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 1,
                text: "v Open title".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 2,
                text: "Visible body".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 3,
                text: "> Details".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 4,
                text: "v Details".to_owned(),
            },
            DisplayCommand::Text {
                x: 0,
                y: 5,
                text: "Fallback body".to_owned(),
            },
        ]
    );
}

#[test]
fn renders_common_form_controls_as_visible_inline_widgets() {
    let render = render_html(
        "mem://form-controls",
        br#"
            <html><body>
              <form>
                <input name="q" value="rust browser">
                <input type="checkbox" checked>
                <input type="radio">
                <input type="password" value="secret">
                <select name="kind">
                  <option value="docs">Docs</option>
                  <option value="news" selected>News</option>
                </select>
                <textarea name="note">ship it</textarea>
                <input type="submit" value="Go">
              </form>
            </body></html>
            "#,
        BrowserRenderOptions {
            width: 120,
            ..BrowserRenderOptions::default()
        },
    );

    assert_eq!(
        render.text,
        "[rust browser] [x] ( ) [******] [News] [ship it] [Go]"
    );
    assert_eq!(
        render.display_list,
        vec![DisplayCommand::Text {
            x: 0,
            y: 0,
            text: "[rust browser] [x] ( ) [******] [News] [ship it] [Go]".to_owned(),
        }]
    );
    assert!(hit_test_target_node(&render, 1, 0).is_some());
    assert_eq!(hit_test_target_node(&render, 14, 0), None);
    assert!(hit_test_target_node(&render, 15, 0).is_some());
}
