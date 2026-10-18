use js_sys::Reflect;
use serde::{Deserialize, Serialize};
use wasm_bindgen::{JsCast as _, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Blob, Document, Element, File, FileSystemDirectoryHandle, FileSystemFileHandle,
    FileSystemGetFileOptions, FileSystemWritableFileStream, HtmlInputElement, Storage,
};

use crate::label::Picture;

const KEY: &str = "b1-labels";
const FIELDS: [&str; 12] = [
    "width",
    "height",
    "text",
    "font-size",
    "font",
    "bold",
    "align",
    "border",
    "raster",
    "density",
    "copies",
    "label-type",
];

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Saved {
    fields: Vec<(String, String)>,
    pub(crate) pictures: Vec<SavedPicture>,
    pub(crate) selected: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SavedPicture {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) size: u16,
    pub(crate) offset: (f64, f64),
    #[serde(default)]
    pub(crate) rotation: i16,
    #[serde(default)]
    pub(crate) darkness: i8,
}

impl Saved {
    pub(crate) fn capture(
        document: &Document,
        pictures: &[Picture],
        selected: Option<usize>,
    ) -> Self {
        Self {
            fields: FIELDS
                .iter()
                .filter_map(|&id| {
                    let value = field_value(&document.get_element_by_id(id)?)?;
                    Some((id.to_owned(), value))
                })
                .collect(),
            pictures: pictures
                .iter()
                .map(|picture| SavedPicture {
                    id: picture.id.clone(),
                    name: picture.name.clone(),
                    size: picture.size,
                    offset: picture.offset,
                    rotation: picture.rotation,
                    darkness: picture.darkness,
                })
                .collect(),
            selected: selected
                .and_then(|index| pictures.get(index))
                .map(|picture| picture.id.clone()),
        }
    }

    pub(crate) fn read(storage: &Storage) -> Self {
        storage
            .get_item(KEY)
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    pub(crate) fn write(&self, storage: &Storage) -> Result<(), JsValue> {
        let json = serde_json::to_string(self).map_err(|error| error.to_string())?;
        storage.set_item(KEY, &json)
    }

    pub(crate) fn restore_fields(&self, document: &Document) {
        for (id, value) in &self.fields {
            if !FIELDS.contains(&id.as_str()) {
                continue;
            }
            let Some(element) = document.get_element_by_id(id) else {
                continue;
            };
            match checkbox(&element) {
                Some(checkbox) => checkbox.set_checked(value == "true"),
                None => {
                    let _restored = Reflect::set(&element, &"value".into(), &value.into());
                }
            }
        }
    }
}

pub(crate) async fn store_file(id: &str, blob: &Blob) -> Result<(), JsValue> {
    let options = FileSystemGetFileOptions::new();
    options.set_create(true);
    let handle: FileSystemFileHandle = JsFuture::from(
        directory()
            .await?
            .get_file_handle_with_options(id, &options),
    )
    .await?
    .dyn_into()?;
    let stream: FileSystemWritableFileStream =
        JsFuture::from(handle.create_writable()).await?.dyn_into()?;
    JsFuture::from(stream.write_with_blob(blob)?).await?;
    JsFuture::from(stream.close()).await?;
    Ok(())
}

pub(crate) async fn load_file(id: &str) -> Result<File, JsValue> {
    let handle: FileSystemFileHandle = JsFuture::from(directory().await?.get_file_handle(id))
        .await?
        .dyn_into()?;
    JsFuture::from(handle.get_file()).await?.dyn_into()
}

pub(crate) async fn delete_file(id: &str) -> Result<(), JsValue> {
    JsFuture::from(directory().await?.remove_entry(id))
        .await
        .map(|_| ())
}

async fn directory() -> Result<FileSystemDirectoryHandle, JsValue> {
    let navigator = web_sys::window().ok_or("no window")?.navigator();
    JsFuture::from(navigator.storage().get_directory())
        .await?
        .dyn_into()
}

fn field_value(element: &Element) -> Option<String> {
    match checkbox(element) {
        Some(checkbox) => Some(checkbox.checked().to_string()),
        None => Reflect::get(element, &"value".into()).ok()?.as_string(),
    }
}

fn checkbox(element: &Element) -> Option<&HtmlInputElement> {
    element
        .dyn_ref::<HtmlInputElement>()
        .filter(|input| input.type_() == "checkbox")
}
