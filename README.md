# TaxGlass Pro

TaxGlass is a modern, intelligent tax return processor designed to streamline the handling of 1040 forms using a sleek glassmorphism interface. It allows users to download standardized Excel templates, upload their tax data, and generate professional PDF tax returns instantly.

## Features

- **Standardized Templates**: Download ready-to-use Excel templates for data entry.
- **Intelligent Processing**: Automatically analyzes uploaded data for completeness.
- **Instant PDF Generation**: Generate a structured tax return PDF from your data.
- **Glassmorphism UI**: A beautiful, modern interface with neon accents and frosted glass effects.
- **Client-Side Security**: All processing happens in the browser, ensuring data privacy.

## Prerequisites

To run this project locally, you only need a modern web browser. No backend server or complex environment is required.

## Quick Start

1. **Clone the repository**:
   ```bash
   git clone https://github.com/hotredsam/Tax-Glass.git
   cd Tax-Glass
   ```

2. **Open the application**:
   Simply open `index.html` in your preferred web browser.

3. **Usage**:
   - Click **Download .xlsx** to get the template.
   - Fill in your tax data in the Excel file.
   - Drag and drop or upload the file to the **Upload Data** section.
   - Click **Download Tax Return PDF** once processing is complete.

## Build for Production

This project is built with vanilla HTML, CSS, and JavaScript. It is ready for production as-is. You can host it on any static web hosting service like GitHub Pages, Vercel, or Netlify.

1. Ensure all paths to `script.js` and `style.css` are correct in `index.html`.
2. Upload the files to your hosting provider.

## Technologies Used

- **HTML5/CSS3**: Custom glassmorphism styling.
- **JavaScript (Vanilla)**: Core logic and data processing.
- **ExcelJS / XLSX**: For handling Excel template downloads and uploads.
- **jsPDF / jsPDF-AutoTable**: For generating professional PDF documents.
