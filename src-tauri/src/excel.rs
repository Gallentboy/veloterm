use calamine::{open_workbook_auto, Data, DataType, Reader, Sheets};
use serde::Serialize;
use std::fs::File;
use std::io::BufReader;

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExcelWorkbookInfo {
    pub sheet_names: Vec<String>,
    pub default_sheet: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExcelSheetData {
    pub sheet_name: String,
    pub total_rows: usize,
    pub total_cols: usize,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// 格式化浮点数，去掉多余的 `.0`
fn format_float(f: f64) -> String {
    if f.is_nan() {
        return "NaN".to_string();
    }
    if f.is_infinite() {
        return if f.is_sign_positive() {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        };
    }
    if f.fract() == 0.0 && f >= (i64::MIN as f64) && f <= (i64::MAX as f64) {
        return (f as i64).to_string();
    }
    let s = f.to_string();
    if let Some(stripped) = s.strip_suffix(".0") {
        stripped.to_string()
    } else {
        s
    }
}

/// 格式化 Excel 日期时间
fn format_excel_datetime(d: &calamine::ExcelDateTime) -> String {
    let (year, month, day, hour, min, sec, milli) = d.to_ymd_hms_milli();
    if d.is_duration() {
        if milli == 0 {
            format!("{hour:02}:{min:02}:{sec:02}")
        } else {
            format!("{hour:02}:{min:02}:{sec:02}.{milli:03}")
        }
    } else if hour == 0 && min == 0 && sec == 0 && milli == 0 {
        format!("{year:04}-{month:02}-{day:02}")
    } else if milli == 0 {
        format!("{year:04}-{month:02}-{day:02} {hour:02}:{min:02}:{sec:02}")
    } else {
        format!("{year:04}-{month:02}-{day:02} {hour:02}:{min:02}:{sec:02}.{milli:03}")
    }
}

/// 转换单元格 DataType 为字符串表示
fn format_cell(cell: &Data) -> String {
    if DataType::is_empty(cell) {
        return String::new();
    }
    match cell {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Float(f) => format_float(*f),
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => {
            if *b {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }
        }
        Data::DateTime(d) => format_excel_datetime(d),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#ERR: {e:?}"),
    }
}

/// 从工作表数据范围中解析出表头与数据行
fn parse_sheet_range(sheet_name: String, range: &calamine::Range<Data>) -> ExcelSheetData {
    let width = range.width();
    if range.is_empty() || width == 0 {
        return ExcelSheetData {
            sheet_name,
            total_rows: 0,
            total_cols: 0,
            headers: Vec::new(),
            rows: Vec::new(),
        };
    }

    let mut rows_iter = range.rows();

    // 寻找首个非空行作为表头
    let mut header_row: Option<&[Data]> = None;
    for row in rows_iter.by_ref() {
        let is_non_empty = row
            .iter()
            .any(|c| !DataType::is_empty(c) && !format_cell(c).trim().is_empty());
        if is_non_empty {
            header_row = Some(row);
            break;
        }
    }

    let headers: Vec<String> = match header_row {
        Some(row) => {
            let mut h = Vec::with_capacity(width);
            for (idx, cell) in row.iter().enumerate() {
                let text = format_cell(cell);
                if text.trim().is_empty() {
                    h.push(format!("Col {}", idx + 1));
                } else {
                    h.push(text);
                }
            }
            while h.len() < width {
                h.push(format!("Col {}", h.len() + 1));
            }
            h
        }
        None => (1..=width).map(|i| format!("Col {i}")).collect(),
    };

    let total_cols = headers.len();

    // 表头之后的所有行作为数据行 rows，上限 10,000 行安全截断，同时记录 total_rows
    const MAX_ROWS: usize = 10_000;
    let mut rows = Vec::new();
    let mut total_rows = 0;

    for row in rows_iter {
        total_rows += 1;
        if rows.len() < MAX_ROWS {
            let row_vec: Vec<String> = row.iter().map(format_cell).collect();
            rows.push(row_vec);
        }
    }

    ExcelSheetData {
        sheet_name,
        total_rows,
        total_cols,
        headers,
        rows,
    }
}

/// 读取指定 Excel 文件的所有工作表元数据
#[tauri::command]
pub fn read_excel_workbook(file_path: String) -> Result<ExcelWorkbookInfo, String> {
    let workbook: Sheets<BufReader<File>> = open_workbook_auto(&file_path)
        .map_err(|e| format!("无法打开 Excel 文件: {e}"))?;

    let sheet_names = workbook.sheet_names();
    if sheet_names.is_empty() {
        return Err("Excel 文件不包含任何有效工作表".to_string());
    }

    let default_sheet = sheet_names.first().cloned().unwrap_or_default();

    Ok(ExcelWorkbookInfo {
        sheet_names,
        default_sheet,
    })
}

/// 读取指定工作表的数据（表头 + 最多 10,000 行数据）
#[tauri::command]
pub fn read_excel_sheet_data(
    file_path: String,
    sheet_name: String,
) -> Result<ExcelSheetData, String> {
    let mut workbook: Sheets<BufReader<File>> = open_workbook_auto(&file_path)
        .map_err(|e| format!("无法打开 Excel 文件: {e}"))?;

    let range = workbook
        .worksheet_range(&sheet_name)
        .map_err(|e| format!("无法加载工作表 '{sheet_name}': {e}"))?;

    Ok(parse_sheet_range(sheet_name, &range))
}

#[cfg(test)]
mod tests {
    use super::*;
    use calamine::{Cell, CellErrorType, ExcelDateTime, ExcelDateTimeType, Range};

    #[test]
    fn test_read_excel_workbook_non_existent() {
        let res = read_excel_workbook("/path/to/non_existent_file.xlsx".to_string());
        assert!(res.is_err());
        let err = res.err().unwrap_or_default();
        assert!(err.contains("无法打开 Excel 文件"));
    }

    #[test]
    fn test_read_excel_sheet_data_non_existent() {
        let res = read_excel_sheet_data(
            "/path/to/non_existent_file.xlsx".to_string(),
            "Sheet1".to_string(),
        );
        assert!(res.is_err());
        let err = res.err().unwrap_or_default();
        assert!(err.contains("无法打开 Excel 文件"));
    }

    #[test]
    fn test_format_float() {
        assert_eq!(format_float(0.0), "0");
        assert_eq!(format_float(42.0), "42");
        assert_eq!(format_float(-42.0), "-42");
        assert_eq!(format_float(3.15), "3.15");
        assert_eq!(format_float(100.5), "100.5");
        assert_eq!(format_float(f64::NAN), "NaN");
        assert_eq!(format_float(f64::INFINITY), "Infinity");
        assert_eq!(format_float(f64::NEG_INFINITY), "-Infinity");
    }

    #[test]
    fn test_format_cell() {
        assert_eq!(format_cell(&Data::Empty), "");
        assert_eq!(format_cell(&Data::String("hello".to_string())), "hello");
        assert_eq!(format_cell(&Data::Int(123)), "123");
        assert_eq!(format_cell(&Data::Float(45.0)), "45");
        assert_eq!(format_cell(&Data::Float(45.67)), "45.67");
        assert_eq!(format_cell(&Data::Bool(true)), "TRUE");
        assert_eq!(format_cell(&Data::Bool(false)), "FALSE");
        assert_eq!(
            format_cell(&Data::DateTimeIso("2026-10-21".to_string())),
            "2026-10-21"
        );
        assert_eq!(
            format_cell(&Data::DurationIso("PT1H30M".to_string())),
            "PT1H30M"
        );
        assert_eq!(
            format_cell(&Data::Error(CellErrorType::Div0)),
            "#ERR: Div0"
        );
    }

    #[test]
    fn test_format_excel_datetime() {
        let dt = ExcelDateTime::new(45943.0, ExcelDateTimeType::DateTime, false);
        let s = format_excel_datetime(&dt);
        assert_eq!(s, "2025-10-13");

        let dt_time = ExcelDateTime::new(45943.5, ExcelDateTimeType::DateTime, false);
        let s_time = format_excel_datetime(&dt_time);
        assert_eq!(s_time, "2025-10-13 12:00:00");
    }

    #[test]
    fn test_parse_sheet_range_normal() {
        let cells = vec![
            Cell::new((0, 0), Data::String("ID".to_string())),
            Cell::new((0, 1), Data::String("Name".to_string())),
            Cell::new((0, 2), Data::String("Age".to_string())),
            Cell::new((1, 0), Data::Int(1)),
            Cell::new((1, 1), Data::String("Alice".to_string())),
            Cell::new((1, 2), Data::Int(28)),
            Cell::new((2, 0), Data::Int(2)),
            Cell::new((2, 1), Data::String("Bob".to_string())),
            Cell::new((2, 2), Data::Int(34)),
        ];
        let range = Range::from_sparse(cells);
        let data = parse_sheet_range("Sheet1".to_string(), &range);

        assert_eq!(data.sheet_name, "Sheet1");
        assert_eq!(data.total_rows, 2);
        assert_eq!(data.total_cols, 3);
        assert_eq!(data.headers, vec!["ID", "Name", "Age"]);
        assert_eq!(data.rows.len(), 2);
        assert_eq!(
            data.rows.first().cloned().unwrap_or_default(),
            vec!["1", "Alice", "28"]
        );
    }

    #[test]
    fn test_parse_sheet_range_fallback_headers() {
        // 第一行作为表头，但第2列为空，第3列为空
        let cells = vec![
            Cell::new((0, 0), Data::String("First".to_string())),
            Cell::new((0, 1), Data::Empty),
            Cell::new((0, 2), Data::String("".to_string())),
            Cell::new((1, 0), Data::Int(10)),
            Cell::new((1, 1), Data::Int(20)),
            Cell::new((1, 2), Data::Int(30)),
        ];
        let range = Range::from_sparse(cells);
        let data = parse_sheet_range("TestSheet".to_string(), &range);

        assert_eq!(data.headers, vec!["First", "Col 2", "Col 3"]);
        assert_eq!(data.total_rows, 1);
        assert_eq!(data.total_cols, 3);
    }

    #[test]
    fn test_parse_sheet_range_empty() {
        let range: Range<Data> = Range::empty();
        let data = parse_sheet_range("Empty".to_string(), &range);

        assert_eq!(data.total_rows, 0);
        assert_eq!(data.total_cols, 0);
        assert!(data.headers.is_empty());
        assert!(data.rows.is_empty());
    }

    #[test]
    fn test_parse_sheet_range_capping() {
        // 创建超过 10,000 行的数据
        let mut cells = vec![Cell::new((0, 0), Data::String("Header".to_string()))];
        for i in 1..=10_050 {
            cells.push(Cell::new((i, 0), Data::Int(i as i64)));
        }
        let range = Range::from_sparse(cells);
        let data = parse_sheet_range("BigSheet".to_string(), &range);

        assert_eq!(data.total_rows, 10_050);
        assert_eq!(data.rows.len(), 10_000);
    }

    #[test]
    fn test_serialization_camel_case() {
        let info = ExcelWorkbookInfo {
            sheet_names: vec!["S1".to_string(), "S2".to_string()],
            default_sheet: "S1".to_string(),
        };
        let json = serde_json::to_string(&info).unwrap_or_default();
        assert!(json.contains("\"sheetNames\":"));
        assert!(json.contains("\"defaultSheet\":"));

        let data = ExcelSheetData {
            sheet_name: "S1".to_string(),
            total_rows: 10,
            total_cols: 2,
            headers: vec!["A".to_string(), "B".to_string()],
            rows: vec![vec!["1".to_string(), "2".to_string()]],
        };
        let json_data = serde_json::to_string(&data).unwrap_or_default();
        assert!(json_data.contains("\"sheetName\":"));
        assert!(json_data.contains("\"totalRows\":"));
        assert!(json_data.contains("\"totalCols\":"));
        assert!(json_data.contains("\"headers\":"));
        assert!(json_data.contains("\"rows\":"));
    }
}
