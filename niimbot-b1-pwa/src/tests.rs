use js_sys::{Array, Promise, Reflect};
use serde_json::Value;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
use web_sys::{
    CanvasRenderingContext2d, DataTransfer, Document, Element, Event, EventInit, File,
    FilePropertyBag, HtmlButtonElement, HtmlCanvasElement, HtmlElement, HtmlInputElement,
    HtmlSelectElement, PointerEvent, PointerEventInit, Storage,
};

wasm_bindgen_test_configure!(run_in_browser);

const PAGE: &str = include_str!("../www/index.html");
const BLACK: &str = "#000";
const WHITE: &str = "#fff";
const GRAY: &str = "#888";

#[wasm_bindgen_test]
async fn editing_text_redraws_the_preview() {
    fresh().await;
    set("text", "");
    assert_eq!(black_dots(), 0);
    set("text", "Hi");
    assert!(black_dots() > 0);
}

#[wasm_bindgen_test]
async fn added_images_all_stay_in_view() {
    fresh().await;
    set("text", "");
    add(&[
        image("base.png", 100, 100, BLACK).await,
        image("left.png", 100, 100, WHITE).await,
        image("right.png", 100, 100, WHITE).await,
    ])
    .await;
    assert_eq!(layers(), ["Text", "right.png", "left.png", "base.png"]);
    assert_eq!(selected_row(), 1);
    assert!(is_black((192, 180)), "base below the quarters");
    assert!(is_black((192, 60)), "base between the quarters");
    assert!(!is_black((100, 60)), "left quarter over the base");
    assert!(!is_black((250, 60)), "right quarter over the base");
}

#[wasm_bindgen_test]
async fn move_up_and_down_restack_the_layers() {
    fresh().await;
    set("text", "");
    add(&[
        image("base.png", 100, 100, BLACK).await,
        image("left.png", 100, 100, WHITE).await,
    ])
    .await;
    assert!(!button("raise-layer").disabled() && !button("lower-layer").disabled());
    assert!(!is_black((100, 60)));

    button("lower-layer").click();
    assert_eq!(layers(), ["Text", "base.png", "left.png"]);
    assert_eq!(selected_row(), 2);
    assert!(is_black((100, 60)), "base now covers left");
    assert!(!button("raise-layer").disabled() && button("lower-layer").disabled());

    reload(2).await;
    assert_eq!(layers(), ["Text", "base.png", "left.png"]);
    assert!(is_black((100, 60)));

    pick_row(2);
    button("raise-layer").click();
    assert_eq!(layers(), ["Text", "left.png", "base.png"]);
    assert_eq!(selected_row(), 1);
    assert!(!is_black((100, 60)));
}

#[wasm_bindgen_test]
async fn pressing_selects_the_top_layer_under_the_pointer() {
    fresh().await;
    set("text", "");
    add(&[
        image("base.png", 100, 100, BLACK).await,
        image("left.png", 100, 100, WHITE).await,
    ])
    .await;
    press((192, 180));
    assert_eq!(selected_row(), 2, "only base lies there");
    press((100, 60));
    assert_eq!(selected_row(), 1, "left lies over base there");
    press((10, 200));
    assert_eq!(selected_row(), 1, "nothing there keeps the selection");
}

#[wasm_bindgen_test]
async fn text_is_a_layer_that_restacks() {
    fresh().await;
    add(&[image("cover.png", 100, 100, WHITE).await]).await;
    assert_eq!(layers(), ["Text", "cover.png"]);
    let text_dots = black_dots();
    assert!(text_dots > 0, "new images land under the text");

    pick_row(0);
    assert!(button("raise-layer").disabled());
    button("lower-layer").click();
    assert_eq!(layers(), ["cover.png", "Text"]);
    assert_eq!(selected_row(), 1);
    assert_eq!(black_dots(), 0, "the white image hides the text");

    button("raise-layer").click();
    assert_eq!(black_dots(), text_dots);
}

#[wasm_bindgen_test]
async fn text_drags_and_disables_the_image_controls() {
    fresh().await;
    let (left, top, right, bottom) = ink_box();
    drag((192, 120), (232, 140));
    assert_eq!(ink_box(), (left + 40, top + 20, right + 40, bottom + 20));
    assert_eq!(saved_text_offset(), (40, 20));
    assert_eq!(selected_row(), 0, "the Text row");

    add(&[image("base.png", 100, 100, GRAY).await]).await;
    assert!(!button("remove-picture").disabled());
    press((232, 140));
    assert_eq!(selected_row(), 0);
    assert!(button("remove-picture").disabled());
    for id in ["picture-size", "picture-rotation", "picture-darkness"] {
        assert!(element::<HtmlInputElement>(id).disabled(), "{id}");
    }
    press((100, 220));
    assert_eq!(selected_row(), 1);
    assert!(!button("remove-picture").disabled());
}

#[wasm_bindgen_test]
async fn dragging_sticks_to_the_centre_lines() {
    fresh().await;
    set("text", "");
    add(&[image("base.png", 100, 100, BLACK).await]).await;
    let centred = preview_hash();
    assert!(guides_hidden());

    pointer("pointerdown", (192, 120));
    pointer("pointermove", (196, 123));
    assert_eq!(preview_hash(), centred, "both lines hold it, no guide dots");
    assert!(guide_shown("guide-vertical") && guide_shown("guide-horizontal"));

    pointer("pointermove", (222, 124));
    assert!(!guide_shown("guide-vertical") && guide_shown("guide-horizontal"));
    pointer("pointerup", (222, 124));
    assert_eq!(saved_offset(0), (30, 0));
    assert!(guides_hidden());
    assert!(is_black((340, 120)) && !is_black((80, 120)));
}

#[wasm_bindgen_test]
async fn left_aligned_text_centres_by_its_block() {
    fresh().await;
    set("align", "left");
    set("text", "Hi");
    let (left, _, right, _) = ink_box();
    drag((u32::midpoint(left, right), 120), (195, 120));
    let centred = preview_hash();
    let (left, _, right, _) = ink_box();
    assert!(
        (left + right).abs_diff(2 * 192) <= 8,
        "ink {left}..{right} sits on the centre line"
    );

    pointer("pointerdown", (195, 120));
    pointer("pointermove", (190, 120));
    assert!(guide_shown("guide-vertical"));
    pointer("pointerup", (190, 120));
    assert_eq!(preview_hash(), centred, "still held on the line");

    drag((190, 120), (180, 120));
    assert_ne!(preview_hash(), centred, "pulled off the line");
    assert!(guides_hidden());
}

#[wasm_bindgen_test]
async fn pinch_resizes_the_selected_image() {
    fresh().await;
    set("text", "");
    add(&[image("base.png", 100, 100, BLACK).await]).await;
    assert!(!is_black((20, 120)));

    finger("pointerdown", 1, (152, 120));
    finger("pointerdown", 2, (232, 120));
    finger("pointermove", 1, (132, 120));
    finger("pointermove", 2, (252, 120));
    assert_eq!(value("picture-size"), "150");
    finger("pointerup", 1, (132, 120));
    finger("pointerup", 2, (252, 120));

    assert_eq!(saved()["pictures"][0]["size"], 150);
    assert_eq!(saved_offset(0), (0, 0));
    assert!(is_black((20, 120)));
}

#[wasm_bindgen_test]
async fn twist_rotates_and_sticks_to_right_angles() {
    fresh().await;
    set("text", "");
    add(&[image("wide.png", 200, 100, BLACK).await]).await;

    finger("pointerdown", 1, (152, 120));
    finger("pointerdown", 2, (232, 120));
    finger("pointermove", 1, (190, 80));
    finger("pointermove", 2, (194, 160));
    finger("pointerup", 1, (190, 80));
    finger("pointerup", 2, (194, 160));
    assert_eq!(picture_settings(), ["100", "90", "0"]);
    assert!(is_black((192, 10)) && !is_black((20, 120)));

    finger("pointerdown", 1, (152, 120));
    finger("pointerdown", 2, (232, 120));
    finger("pointermove", 1, (158, 100));
    finger("pointermove", 2, (227, 140));
    finger("pointerup", 1, (158, 100));
    finger("pointerup", 2, (227, 140));
    assert_eq!(value("picture-rotation"), "120");
    assert_eq!(saved()["pictures"][0]["rotation"], 120);
}

#[wasm_bindgen_test]
async fn two_fingers_move_and_one_finger_carries_on() {
    fresh().await;
    set("text", "");
    add(&[image("base.png", 100, 100, BLACK).await]).await;

    finger("pointerdown", 1, (152, 120));
    finger("pointerdown", 2, (232, 120));
    finger("pointermove", 1, (182, 140));
    finger("pointermove", 2, (262, 140));
    finger("pointerup", 1, (182, 140));
    finger("pointermove", 2, (272, 150));
    finger("pointerup", 2, (272, 150));

    assert_eq!(saved_offset(0), (40, 30));
    assert_eq!(picture_settings(), ["100", "0", "0"]);
}

#[wasm_bindgen_test]
async fn pinch_scales_the_text() {
    fresh().await;
    finger("pointerdown", 1, (192, 120));
    finger("pointerdown", 2, (272, 120));
    finger("pointermove", 2, (312, 120));
    finger("pointerup", 1, (192, 120));
    finger("pointerup", 2, (312, 120));

    assert_eq!(selected_row(), 0, "the Text row");
    assert_eq!(value("font-size"), "84");
    assert!(
        saved()["fields"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!(["font-size", "84"]))
    );
    assert!(element::<HtmlInputElement>("picture-rotation").disabled());
}

#[wasm_bindgen_test]
async fn dragging_stops_layers_at_the_edge() {
    fresh().await;
    set("text", "");
    add(&[image("base.png", 100, 100, BLACK).await]).await;
    drag((192, 120), (900, 700));
    assert_eq!(saved_offset(0), (192, 120));
    assert!(is_black((380, 236)));

    pick_row(0);
    press((380, 236));
    assert_eq!(selected_row(), 1, "the part still showing grabs it");
    drag((380, 236), (192, 120));
    assert_eq!(saved_offset(0), (0, 0));
}

#[wasm_bindgen_test]
async fn text_cannot_leave_the_label() {
    fresh().await;
    drag((192, 120), (900, 700));
    assert_eq!(saved_text_offset(), (192, 120));
    assert!(black_dots() > 0);
}

#[wasm_bindgen_test]
async fn shrinking_the_label_pulls_layers_back() {
    fresh().await;
    set("text", "");
    add(&[image("base.png", 100, 100, BLACK).await]).await;
    drag((192, 120), (342, 120));
    assert_eq!(saved_offset(0), (150, 0));
    set("orientation", "vertical");
    assert_eq!(saved_offset(0), (120, 0));
    assert!(is_black((230, 192)));
}

#[wasm_bindgen_test]
async fn lost_layers_come_back_from_an_old_save() {
    fresh().await;
    let file = image("far.png", 100, 100, BLACK).await;
    crate::saved::store_file("far", &file).await.unwrap();
    storage()
        .set_item(
            "b1-labels",
            r#"{"fields":[["text","Hi"]],"pictures":[{"id":"far","name":"far.png","size":100,"offset":[5000.0,-5000.0]}],"text_offset":[-9000.0,0.0],"selected":"far"}"#,
        )
        .unwrap();

    reload(1).await;
    assert!(is_black((370, 10)), "the image's corner is back");
    let (left, _, _, _) = ink_box();
    assert!(
        left < 30,
        "the text is back at the left edge, ink from {left}"
    );
}

#[wasm_bindgen_test]
async fn center_puts_the_image_back_in_the_middle() {
    fresh().await;
    set("text", "");
    add(&[image("base.png", 100, 100, BLACK).await]).await;
    set("picture-size", "60");
    button("turn-picture").click();
    let centred = preview_hash();
    drag((192, 120), (250, 170));
    assert_eq!(saved_offset(0), (58, 50));

    button("centre-layer").click();
    assert_eq!(saved_offset(0), (0, 0));
    assert_eq!(picture_settings(), ["60", "90", "0"]);
    assert_eq!(preview_hash(), centred);
}

#[wasm_bindgen_test]
async fn center_puts_the_text_back_in_the_middle() {
    fresh().await;
    set("align", "left");
    set("text", "Hi");
    let start = ink_box();
    drag((u32::midpoint(start.0, start.2), 120), (60, 40));
    pick_row(0);
    button("centre-layer").click();

    let (left, top, right, bottom) = ink_box();
    assert!(
        (left + right).abs_diff(2 * 192) <= 8,
        "ink {left}..{right} across the middle"
    );
    assert!(
        (top + bottom).abs_diff(start.1 + start.3) <= 2,
        "ink {top}..{bottom} back on its row"
    );
    assert_eq!(saved_text_offset().1, 0);
}

#[wasm_bindgen_test]
async fn dragging_moves_the_selected_image() {
    fresh().await;
    set("text", "");
    add(&[image("base.png", 100, 100, BLACK).await]).await;
    drag((192, 120), (232, 140));
    assert_eq!(saved_offset(0), (40, 20));
    assert!(is_black((340, 130)) && !is_black((80, 130)));
}

#[wasm_bindgen_test]
async fn settings_follow_the_selected_image() {
    fresh().await;
    add(&[
        image("base.png", 100, 100, BLACK).await,
        image("left.png", 100, 100, WHITE).await,
    ])
    .await;
    assert_eq!(value("picture-size"), "50");
    set("picture-size", "80");
    set("picture-rotation", "45");
    set("picture-darkness", "-4");

    pick_row(2);
    assert_eq!(picture_settings(), ["100", "0", "0"]);
    button("turn-picture").click();
    assert_eq!(value("picture-rotation"), "90");

    pick_row(1);
    assert_eq!(picture_settings(), ["80", "45", "-4"]);
}

#[wasm_bindgen_test]
async fn quarter_turn_fits_a_wide_image_to_the_label_height() {
    fresh().await;
    set("text", "");
    add(&[image("wide.png", 200, 100, BLACK).await]).await;
    assert!(is_black((20, 120)) && !is_black((192, 10)));
    button("turn-picture").click();
    assert!(is_black((140, 120)) && is_black((245, 120)) && is_black((192, 10)));
    assert!(!is_black((120, 120)) && !is_black((260, 120)));
    for _ in 0..3 {
        button("turn-picture").click();
    }
    assert_eq!(value("picture-rotation"), "0");
    assert!(is_black((20, 120)));
}

#[wasm_bindgen_test]
async fn darkness_changes_only_the_selected_image() {
    fresh().await;
    set("text", "");
    add(&[
        image("base.png", 100, 100, GRAY).await,
        image("left.png", 100, 100, GRAY).await,
    ])
    .await;
    assert_eq!(black_dots(), 0, "mid gray prints blank when sharp");
    set("picture-darkness", "10");
    assert!(is_black((100, 60)));
    assert!(!is_black((192, 180)));
    set("picture-darkness", "0");
    assert_eq!(black_dots(), 0);
}

#[wasm_bindgen_test]
async fn remove_takes_only_the_selected_image() {
    fresh().await;
    add(&[
        image("base.png", 100, 100, BLACK).await,
        image("left.png", 100, 100, WHITE).await,
        image("right.png", 100, 100, WHITE).await,
    ])
    .await;
    pick_row(2);
    button("remove-picture").click();
    assert_eq!(layers(), ["Text", "right.png", "base.png"]);
    assert_eq!(selected_row(), 1);

    reload(2).await;
    assert_eq!(layers(), ["Text", "right.png", "base.png"]);
}

#[wasm_bindgen_test]
async fn vertical_label_previews_upright() {
    fresh().await;
    set("text", "");
    set("orientation", "vertical");
    assert_eq!(preview_size(), (240, 384));
    assert_eq!(text("size"), "50 x 30 mm, 384 x 240 dots");

    add(&[image("wide.png", 200, 100, BLACK).await]).await;
    assert!(is_black((120, 140)) && !is_black((120, 120)));
    drag((120, 192), (120, 242));
    assert_eq!(saved_offset(0), (0, 50));
    assert!(is_black((120, 290)) && !is_black((120, 170)));
}

#[wasm_bindgen_test]
async fn reload_restores_the_label() {
    fresh().await;
    set("width", "40");
    set("text", "Hi");
    set("raster", "dither");
    set("orientation", "vertical");
    add(&[
        image("base.png", 100, 100, GRAY).await,
        image("left.png", 60, 100, BLACK).await,
    ])
    .await;
    drag((60, 80), (70, 70));
    pick_row(2);
    set("picture-size", "80");
    button("turn-picture").click();
    set("picture-darkness", "3");
    drag((120, 160), (120, 200));
    button("lower-layer").click();
    let before = preview_hash();

    reload(2).await;
    assert_eq!(
        [
            value("width"),
            value("text"),
            value("raster"),
            value("orientation")
        ],
        ["40", "Hi", "dither", "vertical"]
    );
    assert_eq!(layers(), ["left.png", "Text", "base.png"]);
    assert_eq!(selected_row(), 1);
    assert_eq!(saved_text_offset(), (0, 40));
    assert_eq!(preview_hash(), before);
    pick_row(2);
    assert_eq!(picture_settings(), ["80", "90", "3"]);
}

#[wasm_bindgen_test]
async fn label_saved_before_rotation_and_orientation_opens() {
    fresh().await;
    let file = image("old.png", 100, 100, WHITE).await;
    crate::saved::store_file("old", &file).await.unwrap();
    storage()
        .set_item(
            "b1-labels",
            r#"{"fields":[["width","50"],["height","30"],["text","Hi"]],"pictures":[{"id":"old","name":"old.png","size":100,"offset":[0.0,0.0]}],"selected":"old"}"#,
        )
        .unwrap();

    reload(1).await;
    assert_eq!(layers(), ["Text", "old.png"]);
    assert_eq!(value("orientation"), "horizontal");
    assert_eq!(
        [value("picture-rotation"), value("picture-darkness")],
        ["0", "0"]
    );
    assert!(black_dots() > 0, "the text over the white image");
}

#[wasm_bindgen_test]
async fn unreadable_file_is_named_in_an_error() {
    fresh().await;
    let options = FilePropertyBag::new();
    options.set_type("image/png");
    let file = File::new_with_str_sequence_and_options(
        &Array::of1(&"not an image".into()),
        "notes.png",
        &options,
    )
    .unwrap();
    choose("add-picture", &[file]);
    wait_for("the error", || {
        element::<HtmlElement>("status").class_name() == "error"
    })
    .await;
    assert!(text("status").contains("notes.png"));
    assert_eq!(layers(), ["Text"]);
}

#[wasm_bindgen_test]
async fn unknown_firmware_is_refused() {
    fresh().await;
    assert!(button("flash").disabled());
    let file =
        File::new_with_str_sequence(&Array::of1(&"not firmware".into()), "random.bin").unwrap();
    choose("firmware-file", &[file]);
    wait_for("the refusal", || {
        element::<HtmlElement>("status").class_name() == "error"
    })
    .await;
    assert!(text("status").contains("random.bin is not a known B1 firmware image"));
    assert!(button("flash").disabled());
}

#[wasm_bindgen_test]
async fn fields_are_large_enough_for_ios_not_to_zoom() {
    fresh().await;
    let style = page_style();
    let fields = document()
        .query_selector_all(
            "#page-under-test input[type=number], #page-under-test select, #page-under-test textarea",
        )
        .unwrap();
    assert!(fields.length() >= 12, "{} fields", fields.length());
    let window = web_sys::window().unwrap();
    for index in 0..fields.length() {
        let field: Element = fields.item(index).unwrap().unchecked_into();
        let size = window
            .get_computed_style(&field)
            .unwrap()
            .unwrap()
            .get_property_value("font-size")
            .unwrap();
        let pixels: f64 = size.trim_end_matches("px").parse().unwrap();
        assert!(pixels >= 16.0, "#{} has {size}", field.id());
    }
    style.remove();
}

#[wasm_bindgen_test]
async fn empty_label_disables_the_image_controls() {
    fresh().await;
    assert!(element::<HtmlElement>("layers").hidden());
    for id in [
        "remove-picture",
        "turn-picture",
        "raise-layer",
        "lower-layer",
        "centre-layer",
    ] {
        assert!(button(id).disabled(), "{id}");
    }
    for id in ["picture-size", "picture-rotation", "picture-darkness"] {
        assert!(element::<HtmlInputElement>(id).disabled(), "{id}");
    }
    let before = preview_hash();
    drag((20, 20), (60, 60));
    assert_eq!(preview_hash(), before);
}

async fn fresh() {
    storage().clear().unwrap();
    open();
    wait_for("the text layer", || layers() == ["Text"]).await;
}

async fn reload(images: usize) {
    open();
    wait_for("the images to come back", || layers().len() == images + 1).await;
}

fn open() {
    let markup = PAGE
        .split_once("<body>")
        .and_then(|(_, rest)| rest.split_once("<script"))
        .map(|(markup, _)| markup)
        .unwrap();
    let document = document();
    for earlier in ["page-under-test", "page-style"] {
        if let Some(earlier) = document.get_element_by_id(earlier) {
            earlier.remove();
        }
    }
    let page = document.create_element("div").unwrap();
    page.set_id("page-under-test");
    page.set_inner_html(markup);
    document.body().unwrap().append_child(&page).unwrap();
    crate::start().unwrap();
}

fn page_style() -> Element {
    let css = PAGE
        .split_once("<style>")
        .and_then(|(_, rest)| rest.split_once("</style>"))
        .map(|(css, _)| css)
        .unwrap();
    let style = document().create_element("style").unwrap();
    style.set_id("page-style");
    style.set_text_content(Some(css));
    document().body().unwrap().append_child(&style).unwrap();
    style
}

async fn add(files: &[File]) {
    let expected = layers().len() + files.len();
    choose("add-picture", files);
    wait_for("the images to load", || layers().len() == expected).await;
}

fn choose(input: &str, files: &[File]) {
    let transfer = DataTransfer::new().unwrap();
    for file in files {
        transfer.items().add_with_file(file).unwrap();
    }
    let input: HtmlInputElement = element(input);
    input.set_files(transfer.files().as_ref());
    input
        .dispatch_event(&Event::new("change").unwrap())
        .unwrap();
}

async fn image(name: &str, width: u32, height: u32, color: &str) -> File {
    let canvas: HtmlCanvasElement = document()
        .create_element("canvas")
        .unwrap()
        .unchecked_into();
    canvas.set_width(width);
    canvas.set_height(height);
    let context: CanvasRenderingContext2d =
        canvas.get_context("2d").unwrap().unwrap().unchecked_into();
    context.set_fill_style_str(color);
    context.fill_rect(0.0, 0.0, f64::from(width), f64::from(height));
    let blob = JsFuture::from(Promise::new(&mut |resolve, _| {
        canvas.to_blob(&resolve).unwrap();
    }))
    .await
    .unwrap();
    let options = FilePropertyBag::new();
    options.set_type("image/png");
    File::new_with_blob_sequence_and_options(&Array::of1(&blob), name, &options).unwrap()
}

fn set(id: &str, value: &str) {
    let field: HtmlElement = element(id);
    Reflect::set(&field, &"value".into(), &value.into()).unwrap();
    let init = EventInit::new();
    init.set_bubbles(true);
    field
        .dispatch_event(&Event::new_with_event_init_dict("input", &init).unwrap())
        .unwrap();
}

fn pick_row(row: i32) {
    let list: HtmlSelectElement = element("layer-list");
    list.set_selected_index(row);
    list.dispatch_event(&Event::new("change").unwrap()).unwrap();
}

fn guide_shown(id: &str) -> bool {
    !element::<HtmlElement>(id).hidden()
}

fn guides_hidden() -> bool {
    !guide_shown("guide-vertical") && !guide_shown("guide-horizontal")
}

fn press(point: (u32, u32)) {
    pointer("pointerdown", point);
    pointer("pointerup", point);
}

fn drag(from: (u32, u32), to: (u32, u32)) {
    pointer("pointerdown", from);
    pointer("pointermove", to);
    pointer("pointerup", to);
}

fn pointer(kind: &str, point: (u32, u32)) {
    finger(kind, 1, point);
}

fn finger(kind: &str, id: i32, (x, y): (u32, u32)) {
    let preview: HtmlCanvasElement = element("preview");
    let bounds = preview.get_bounding_client_rect();
    let pixels_per_dot = bounds.width() / f64::from(preview.width());
    let init = PointerEventInit::new();
    init.set_bubbles(true);
    init.set_pointer_id(id);
    init.set_is_primary(id == 1);
    init.set_pointer_type("touch");
    init.set_client_x(whole(bounds.left() + f64::from(x) * pixels_per_dot));
    init.set_client_y(whole(bounds.top() + f64::from(y) * pixels_per_dot));
    preview
        .dispatch_event(&PointerEvent::new_with_event_init_dict(kind, &init).unwrap())
        .unwrap();
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "coordinates on the preview are a few hundred dots"
)]
fn whole(coordinate: f64) -> i32 {
    coordinate.round() as i32
}

fn is_black((x, y): (u32, u32)) -> bool {
    let (width, height) = preview_size();
    assert!(x < width && y < height, "({x}, {y}) is off the preview");
    preview_pixels()[usize::try_from((y * width + x) * 4).unwrap()] == 0
}

fn ink_box() -> (u32, u32, u32, u32) {
    let (width, _) = preview_size();
    let mut bounds = (u32::MAX, u32::MAX, 0, 0);
    for (index, pixel) in preview_pixels().as_chunks::<4>().0.iter().enumerate() {
        if pixel[0] == 0 {
            let index = u32::try_from(index).unwrap();
            let (x, y) = (index % width, index / width);
            bounds = (
                bounds.0.min(x),
                bounds.1.min(y),
                bounds.2.max(x),
                bounds.3.max(y),
            );
        }
    }
    bounds
}

fn black_dots() -> usize {
    preview_pixels()
        .iter()
        .step_by(4)
        .filter(|&&level| level == 0)
        .count()
}

fn preview_hash() -> String {
    let (width, height) = preview_size();
    let mut hash: u32 = 0x811c_9dc5;
    for (index, pixel) in preview_pixels().as_chunks::<4>().0.iter().enumerate() {
        if pixel[0] == 0 {
            hash = (hash ^ u32::try_from(index).unwrap()).wrapping_mul(0x0100_0193);
        }
    }
    format!("{width}x{height} {}:{hash:08x}", black_dots())
}

fn preview_pixels() -> Vec<u8> {
    let preview: HtmlCanvasElement = element("preview");
    let context: CanvasRenderingContext2d =
        preview.get_context("2d").unwrap().unwrap().unchecked_into();
    let (width, height) = preview_size();
    context
        .get_image_data(
            0,
            0,
            i32::try_from(width).unwrap(),
            i32::try_from(height).unwrap(),
        )
        .unwrap()
        .data()
        .0
}

fn preview_size() -> (u32, u32) {
    let preview: HtmlCanvasElement = element("preview");
    (preview.width(), preview.height())
}

fn layers() -> Vec<String> {
    let list: HtmlSelectElement = element("layer-list");
    (0..list.length())
        .filter_map(|index| list.item(index)?.text_content())
        .collect()
}

fn selected_row() -> i32 {
    element::<HtmlSelectElement>("layer-list").selected_index()
}

fn saved_offset(index: usize) -> (i32, i32) {
    whole_pair(&saved()["pictures"][index]["offset"])
}

fn saved_text_offset() -> (i32, i32) {
    whole_pair(&saved()["text_offset"])
}

fn whole_pair(pair: &Value) -> (i32, i32) {
    let axis = |axis: usize| whole(pair[axis].as_f64().unwrap());
    (axis(0), axis(1))
}

fn saved() -> Value {
    serde_json::from_str(&storage().get_item("b1-labels").unwrap().unwrap()).unwrap()
}

fn picture_settings() -> [String; 3] {
    [
        value("picture-size"),
        value("picture-rotation"),
        value("picture-darkness"),
    ]
}

fn value(id: &str) -> String {
    Reflect::get(&element::<HtmlElement>(id), &"value".into())
        .unwrap()
        .as_string()
        .unwrap()
}

fn text(id: &str) -> String {
    element::<HtmlElement>(id)
        .text_content()
        .unwrap_or_default()
}

fn button(id: &str) -> HtmlButtonElement {
    element(id)
}

fn element<T: JsCast>(id: &str) -> T {
    document()
        .get_element_by_id(id)
        .unwrap_or_else(|| panic!("missing #{id}"))
        .unchecked_into()
}

async fn wait_for(what: &str, ready: impl Fn() -> bool) {
    for _ in 0..200 {
        if ready() {
            return;
        }
        sleep(10).await;
    }
    panic!("timed out waiting for {what}");
}

async fn sleep(milliseconds: i32) {
    let window = web_sys::window().unwrap();
    JsFuture::from(Promise::new(&mut |resolve, _| {
        window
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, milliseconds)
            .unwrap();
    }))
    .await
    .unwrap();
}

fn document() -> Document {
    web_sys::window().unwrap().document().unwrap()
}

fn storage() -> Storage {
    web_sys::window().unwrap().local_storage().unwrap().unwrap()
}
