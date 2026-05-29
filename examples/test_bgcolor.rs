use umya_spreadsheet::reader;

fn main() {
    let path = std::env::args().nth(1).expect("Usage: test_bgcolor <xlsx_path>");
    let book = reader::xlsx::read(&path).unwrap();
    let theme = book.get_theme();

    for worksheet in book.get_sheet_collection().iter() {
        for (col, row) in [(1, 1), (2, 1), (1, 14), (3, 1)] {
            let style = worksheet.get_style((col, row));
            if let Some(bg_color) = style.get_background_color() {
                let argb = bg_color.get_argb();
                let resolved = bg_color.get_argb_with_theme(theme);
                println!("({},{}) argb='{}' resolved='{}'", col, row, argb, resolved);
                if !resolved.is_empty() {
                    let r = u8::from_str_radix(&resolved[resolved.len()-6..resolved.len()-4], 16).unwrap();
                    let g = u8::from_str_radix(&resolved[resolved.len()-4..resolved.len()-2], 16).unwrap();
                    let b = u8::from_str_radix(&resolved[resolved.len()-2..], 16).unwrap();
                    println!("  → RGB({},{},{}) = #{:02X}{:02X}{:02X}", r, g, b, r, g, b);
                }
            }
        }
        break;
    }
}
