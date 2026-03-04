# TaxGlass Pro

A client-side tax return processor with a glassmorphism UI. Users download a standardized Excel template, fill in their 1040 data, upload it back, and the app generates a formatted PDF tax return. All processing happens in the browser -- no server, no backend, no data leaves the machine.

## Tech Stack

| Technology         | Version   | Purpose                                  |
|--------------------|-----------|------------------------------------------|
| HTML5              | --        | Page structure                           |
| CSS3               | --        | Glassmorphism styling with CSS variables |
| JavaScript (ES6+)  | Vanilla   | All application logic                    |
| ExcelJS            | 4.4.0     | Template generation (styled .xlsx)       |
| SheetJS (xlsx)     | 0.18.5    | Reading uploaded Excel files             |
| jsPDF              | 2.5.1     | PDF generation                           |
| jsPDF-AutoTable    | 3.5.25    | PDF table formatting                     |

No Node.js, no npm, no build step. Open `index.html` in a browser.

## File Structure

```
TaxGlass/
  index.html       # Single-page application entry
  script.js        # All JS logic: template generation, file upload, PDF rendering
  style.css        # Glassmorphism theme with CSS custom properties
  README.md        # Project overview
```

That is the entire application. Three files.

## How to Run

1. Open `index.html` in any modern browser (Chrome, Firefox, Edge, Safari).
2. Click "Download .xlsx" to get the 1040 template.
3. Fill in tax data in Excel and save.
4. Drag the file onto the upload area (or click to browse).
5. Click "Download Tax Return PDF" to generate the return.

There is no build step, no dev server, and no dependencies to install. All libraries are loaded from CDN links in `index.html`.

## How It Works

### Template Generation (ExcelJS)
- `downloadBtn` click handler creates an in-memory ExcelJS workbook.
- Adds a "1040 Data Entry" sheet with 4 columns: Form Section, Line/Field Name, Value, Instructions.
- Pre-fills 25 rows covering PERSONAL, ADDRESS, QUESTIONS, INCOME, DEDUCTIONS, and TAX/PAYMENTS sections.
- Adds dropdown data validations for Filing Status (5 options) and Yes/No fields.
- Exports as a downloadable `.xlsx` blob.

### File Upload (SheetJS)
- Handles both `<input type="file">` and drag-and-drop.
- Validates file type (.xlsx/.xls) and size (max 10MB).
- Reads with `XLSX.read()`, converts first sheet to JSON with `XLSX.utils.sheet_to_json()`.
- Parses rows into a `taxData` object keyed by "Line / Field Name".
- Tracks missing fields in a `missingItems` array.

### PDF Generation (jsPDF)
- Creates an A4 portrait PDF formatted like a real Form 1040.
- Sections: header with form number and year, filing status, personal info, income lines (1z through 7), calculated totals (Total Income, AGI, Taxable Income), payments/credits, and signature area.
- Missing fields render as red `[ MISSING ]` with highlighted backgrounds.
- Calculates: totalIncome, adjustments, AGI, deduction, taxableIncome.
- Tax year is automatically set to `currentYear - 1`.

## Code Style

- Vanilla JavaScript with no framework.
- All logic wrapped in a single `DOMContentLoaded` event listener.
- CSS custom properties for theming (`--glass-bg`, `--neon-cyan`, `--neon-violet`).
- Glassmorphism pattern: `backdrop-filter: blur(20px)`, translucent backgrounds, animated blob gradients.
- Functions are declared inside the event listener scope (not global).
- HTML escaping via `escapeHtml()` helper to prevent XSS in error messages.

## Important Files -- Do Not Modify Carelessly

- `index.html` lines 8-11 -- CDN script tags. Changing versions or removing them breaks all functionality.
- `script.js` template rows (lines 51-78) -- These define the exact column names the parser expects. The upload parser reads `"Line / Field Name"` and `"Value"` columns by those exact header strings. Renaming headers in the template without updating the parser will silently break uploads.
- `style.css` `.hidden` class -- Used by JS to toggle section visibility. Do not remove or rename.

## Gotchas and Warnings

- Do NOT add a bundler or build step unless you also update the CDN script tags to local imports. The current setup loads ExcelJS, SheetJS, jsPDF, and jsPDF-AutoTable from CDNs at runtime.
- Do NOT change the Excel column headers ("Form Section", "Line / Field Name", "Value", "Instructions / Options") without updating `processTaxData()` in script.js. The parser uses those exact strings.
- Do NOT rely on the `generatePdfBtn` click handler wired in `index.html` -- it gets re-wired dynamically after processing in `processTaxData()` because the success card HTML is replaced via innerHTML.
- The standard deduction is hardcoded to $13,850 (2023 single). This needs annual updates.
- The `TAX_YEAR` is computed as `new Date().getFullYear() - 1`. This is correct for typical filing but is not configurable.
- There are no tests. If you add tests, consider using a browser-based runner (Playwright, Cypress) since the app depends on browser APIs (FileReader, Blob, DOM).
- The body has `overflow: hidden` which prevents scrolling. On small screens, content may be clipped.
