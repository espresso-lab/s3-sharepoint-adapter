use crate::config;

use super::azure::SharePointObjects;
use regex::Regex;
use std::io::Cursor;
use xml::writer::XmlEvent;
use xml::EmitterConfig;

pub fn generate_s3_list_objects_v2_response(
    bucket: String,
    prefix: String,
    objects: SharePointObjects,
    files_only: bool,
) -> String {
    let filename_pattern = config().filename_pattern.clone();
    let regex = Regex::new(&filename_pattern).unwrap();
    let mut buffer = Cursor::new(Vec::new());
    let mut writer = EmitterConfig::new()
        .perform_indent(true)
        .create_writer(&mut buffer);

    writer
        .write(XmlEvent::start_element("ListBucketResult"))
        .unwrap();

    writer.write(XmlEvent::start_element("Name")).unwrap();
    writer.write(XmlEvent::characters(&bucket)).unwrap();
    writer.write(XmlEvent::end_element()).unwrap(); // Name

    writer.write(XmlEvent::start_element("Prefix")).unwrap();
    writer
        .write(XmlEvent::characters(&format!(
            "{}/",
            &prefix.trim_start_matches("/")
        )))
        .unwrap();
    writer.write(XmlEvent::end_element()).unwrap(); // Prefix

    writer
        .write(XmlEvent::start_element("IsTruncated"))
        .unwrap();
    writer.write(XmlEvent::characters("false")).unwrap();
    writer.write(XmlEvent::end_element()).unwrap(); // IsTruncated

    writer.write(XmlEvent::start_element("MaxKeys")).unwrap();
    writer.write(XmlEvent::characters("1000")).unwrap();
    writer.write(XmlEvent::end_element()).unwrap(); // MaxKeys

    writer.write(XmlEvent::start_element("Marker")).unwrap();
    writer.write(XmlEvent::characters("")).unwrap();
    writer.write(XmlEvent::end_element()).unwrap(); // Marker

    if !files_only {
        for folder in objects.items.iter().filter(|item| item.folder.is_some()) {
            writer
                .write(XmlEvent::start_element("CommonPrefixes"))
                .unwrap();
            writer.write(XmlEvent::start_element("Prefix")).unwrap();
            writer
                .write(XmlEvent::characters(&format!(
                    "{}/{}/",
                    &prefix.trim_start_matches("/"),
                    &folder.name
                )))
                .unwrap();
            writer.write(XmlEvent::end_element()).unwrap(); // Prefix
            writer.write(XmlEvent::end_element()).unwrap(); // CommonPrefixes
        }
    }

    writer.write(XmlEvent::start_element("Contents")).unwrap();

    writer.write(XmlEvent::start_element("Key")).unwrap();
    writer
        .write(XmlEvent::characters(&format!(
            "{}/",
            &prefix.trim_start_matches("/")
        )))
        .unwrap();
    writer.write(XmlEvent::end_element()).unwrap(); // Key

    writer.write(XmlEvent::start_element("Size")).unwrap();
    writer.write(XmlEvent::characters("0")).unwrap();
    writer.write(XmlEvent::end_element()).unwrap(); // Size

    writer.write(XmlEvent::end_element()).unwrap(); // Contents

    for item in objects
        .items
        .iter()
        .filter(|item| item.file.is_some() && regex.is_match(&item.name.to_lowercase()))
    {
        writer.write(XmlEvent::start_element("Contents")).unwrap();

        writer.write(XmlEvent::start_element("Key")).unwrap();
        writer
            .write(XmlEvent::characters(&format!(
                "{}/{}",
                &prefix.trim_start_matches("/"),
                &item.name
            )))
            .unwrap();
        writer.write(XmlEvent::end_element()).unwrap(); // Key

        writer.write(XmlEvent::start_element("Size")).unwrap();
        writer
            .write(XmlEvent::characters(&item.size.unwrap_or(0).to_string()))
            .unwrap();
        writer.write(XmlEvent::end_element()).unwrap(); // Size

        writer
            .write(XmlEvent::start_element("LastModified"))
            .unwrap();
        writer
            .write(XmlEvent::characters(
                &item
                    .last_modified_date_time
                    .clone()
                    .unwrap_or("".to_string()),
            ))
            .unwrap();
        writer.write(XmlEvent::end_element()).unwrap(); // LastModified

        writer.write(XmlEvent::start_element("ETag")).unwrap();
        writer
            .write(XmlEvent::characters(
                &item.e_tag.clone().unwrap_or("".to_string()),
            ))
            .unwrap();
        writer.write(XmlEvent::end_element()).unwrap(); // ETag

        writer
            .write(XmlEvent::start_element("StorageClass"))
            .unwrap();
        writer.write(XmlEvent::characters("STANDARD")).unwrap();
        writer.write(XmlEvent::end_element()).unwrap(); // StorageClass

        writer.write(XmlEvent::end_element()).unwrap(); // Contents
    }

    writer.write(XmlEvent::end_element()).unwrap(); // ListBucketResult

    String::from_utf8(buffer.into_inner()).unwrap()
}
