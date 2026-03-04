document.addEventListener('DOMContentLoaded', () => {
    const downloadBtn = document.getElementById('downloadBtn');
    const fileInput = document.getElementById('fileInput');
    const dropZone = document.getElementById('dropZone');
    const statusSection = document.getElementById('statusSection');
    const resultSection = document.getElementById('resultSection');
    const missingCountSpan = document.getElementById('missingCount');
    const generatePdfBtn = document.getElementById('generatePdfBtn');

    let taxData = {};
    let missingItems = [];
    const TAX_YEAR = new Date().getFullYear() - 1;

    function showError(message) {
        statusSection.classList.add('hidden');
        resultSection.classList.remove('hidden');
        missingCountSpan.closest('.success-card').innerHTML =
            '<h3 style="color:#ff4d4d">Error</h3>' +
            '<p>' + escapeHtml(message) + '</p>' +
            '<button class="btn secondary" onclick="location.reload()">Try Again</button>';
    }

    function escapeHtml(str) {
        const div = document.createElement('div');
        div.textContent = str;
        return div.innerHTML;
    }

    // --- 1. Advanced Template Generation (ExcelJS) ---
    downloadBtn.addEventListener('click', async () => {
        const workbook = new ExcelJS.Workbook();
        const sheet = workbook.addWorksheet('1040 Data Entry');

        // Styles
        const headerStyle = {
            font: { bold: true, color: { argb: 'FFFFFFFF' } },
            fill: { type: 'pattern', pattern: 'solid', fgColor: { argb: 'FF444444' } },
            alignment: { horizontal: 'center' }
        };

        // Columns
        sheet.columns = [
            { header: 'Form Section', key: 'section', width: 25 },
            { header: 'Line / Field Name', key: 'field', width: 35 },
            { header: 'Value', key: 'value', width: 30 },
            { header: 'Instructions / Options', key: 'note', width: 40 }
        ];

        sheet.getRow(1).eachCell(cell => Object.assign(cell, headerStyle));

        const rows = [
            ['PERSONAL', 'Filing Status', 'Single', 'Dropdown: Single, Married Filing Jointly...'],
            ['PERSONAL', 'First Name', '', 'Enter legal first name'],
            ['PERSONAL', 'Last Name', '', 'Enter legal last name'],
            ['PERSONAL', 'SSN', '', 'Format: 000-00-0000'],
            ['PERSONAL', 'Spouse First Name', '', 'If applicable'],
            ['PERSONAL', 'Spouse Last Name', '', 'If applicable'],
            ['PERSONAL', 'Spouse SSN', '', 'If applicable'],
            ['ADDRESS', 'Street Address', '', ''],
            ['ADDRESS', 'City', '', ''],
            ['ADDRESS', 'State', 'NY', '2-Letter Code'],
            ['ADDRESS', 'ZIP Code', '', ''],
            ['QUESTIONS', 'Presidential Election Fund', 'No', 'Yes / No'],
            ['QUESTIONS', 'Virtual Currency', 'No', 'Yes / No (Did you receive/sell?)'],
            ['INCOME', '1z: Wages, salaries, tips', 0, 'From W-2 Box 1'],
            ['INCOME', '2b: Taxable Interest', 0, 'From 1099-INT'],
            ['INCOME', '3b: Ordinary Dividends', 0, 'From 1099-DIV'],
            ['INCOME', '4b: IRA distributions', 0, 'Taxable amount'],
            ['INCOME', '5b: Pensions and annuities', 0, 'Taxable amount'],
            ['INCOME', '6b: Social security benefits', 0, 'Taxable amount'],
            ['INCOME', '7: Capital gain or (loss)', 0, 'Schedule D if required'],
            ['DEDUCTIONS', '10: Adjustments to Income', 0, 'From Schedule 1, Line 26'],
            ['DEDUCTIONS', '12: Standard/Itemized Deduction', 13850, '2023 Single: $13,850 | MFJ: $27,700'],
            ['DEDUCTIONS', '13: Qualified Business Income', 0, 'Form 8995'],
            ['TAX/PAYMENTS', '25: Federal Income Tax Withheld', 0, 'From W-2 / 1099'],
            ['TAX/PAYMENTS', '27: Earned Income Credit (EIC)', 0, 'If qualifying'],
            ['TAX/PAYMENTS', '31: Amount from Form 8812', 0, 'Child Tax Credit']
        ];

        sheet.addRows(rows);

        // Add Data Validations (Dropdowns)
        sheet.getCell('C2').dataValidation = {
            type: 'list',
            allowBlank: false,
            formulae: ['"Single,Married Filing Jointly,Married Filing Separately,Head of Household,Qualifying Surviving Spouse"']
        };

        ['C13', 'C14'].forEach(ref => {
            sheet.getCell(ref).dataValidation = {
                type: 'list',
                allowBlank: false,
                formulae: ['"Yes,No"']
            };
        });

        // Styling the 'Value' column to look editable
        sheet.getColumn(3).eachCell((cell, rowNumber) => {
            if (rowNumber > 1) {
                cell.fill = { type: 'pattern', pattern: 'solid', fgColor: { argb: 'FFFFFFE0' } };
                cell.border = { top: {style:'thin'}, left: {style:'thin'}, bottom: {style:'thin'}, right: {style:'thin'} };
            }
        });

        const buffer = await workbook.xlsx.writeBuffer();
        const blob = new Blob([buffer], { type: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet' });
        const url = window.URL.createObjectURL(blob);
        const anchor = document.createElement('a');
        anchor.href = url;
        anchor.download = '1040_Tax_Template_Pro.xlsx';
        anchor.click();
        window.URL.revokeObjectURL(url);
    });

    // --- 2. File Upload & Processing ---
    fileInput.addEventListener('change', handleFile);

    // Drag-and-drop support
    dropZone.addEventListener('dragover', (e) => {
        e.preventDefault();
        dropZone.classList.add('drag-over');
    });
    dropZone.addEventListener('dragleave', () => {
        dropZone.classList.remove('drag-over');
    });
    dropZone.addEventListener('drop', (e) => {
        e.preventDefault();
        dropZone.classList.remove('drag-over');
        const file = e.dataTransfer.files[0];
        if (file) processFile(file);
    });

    function handleFile(e) {
        const file = e.target.files[0];
        if (file) processFile(file);
    }

    function processFile(file) {
        // Validate file type
        const validTypes = [
            'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
            'application/vnd.ms-excel'
        ];
        if (!validTypes.includes(file.type) && !file.name.match(/\.xlsx?$/i)) {
            showError('Invalid file type. Please upload an .xlsx or .xls file.');
            return;
        }
        // Validate file size (max 10 MB)
        if (file.size > 10 * 1024 * 1024) {
            showError('File is too large. Maximum size is 10 MB.');
            return;
        }

        statusSection.classList.remove('hidden');
        resultSection.classList.add('hidden');

        const reader = new FileReader();
        reader.onload = (evt) => {
            try {
                const data = new Uint8Array(evt.target.result);
                const workbook = XLSX.read(data, { type: 'array' });
                if (!workbook.SheetNames.length) {
                    showError('The uploaded file contains no sheets.');
                    return;
                }
                const sheet = workbook.Sheets[workbook.SheetNames[0]];
                const json = XLSX.utils.sheet_to_json(sheet);
                if (!json.length) {
                    showError('The uploaded sheet is empty. Please use the TaxGlass template.');
                    return;
                }
                processTaxData(json);
            } catch (err) {
                showError('Failed to read the file. Ensure it is a valid Excel file.');
            }
        };
        reader.onerror = () => {
            showError('An error occurred while reading the file. Please try again.');
        };
        reader.readAsArrayBuffer(file);
    }

    function processTaxData(data) {
        taxData = {};
        missingItems = [];

        data.forEach(row => {
            const field = row['Line / Field Name'];
            const value = row['Value'];
            taxData[field] = value;

            if (value === undefined || value === "" || value === null) {
                missingItems.push(field);
            }
        });

        // Brief delay for visual feedback (processing is synchronous)
        setTimeout(() => {
            statusSection.classList.add('hidden');
            resultSection.classList.remove('hidden');
            const successCard = resultSection.querySelector('.success-card');
            successCard.innerHTML =
                '<h3>Processing Complete!</h3>' +
                '<p>We\'ve detected <span id="missingCount">' + missingItems.length + '</span> missing items.</p>' +
                '<button id="generatePdfBtn" class="btn primary">Download Tax Return PDF</button>';
            document.getElementById('generatePdfBtn').addEventListener('click', generatePdf);
        }, 800);
    }

    // --- 3. Form 1040 PDF Generator ---
    generatePdfBtn.addEventListener('click', () => {
        const { jsPDF } = window.jspdf;
        const doc = new jsPDF('p', 'mm', 'a4');
        
        const drawHeader = () => {
            doc.setFont("helvetica", "bold");
            doc.setFontSize(14);
            doc.rect(10, 10, 30, 15);
            doc.text("Form 1040", 12, 18);
            
            doc.setFontSize(10);
            doc.text("U.S. Individual Income Tax Return", 105, 15, { align: 'center' });
            doc.text(String(TAX_YEAR), 185, 18);
            doc.line(10, 26, 200, 26);
        };

        const drawSection = (title, y) => {
            doc.setFillColor(240, 240, 240);
            doc.rect(10, y, 190, 6, 'F');
            doc.setFontSize(9);
            doc.setTextColor(0);
            doc.text(title, 12, y + 4.5);
            return y + 6;
        };

        const drawField = (label, value, x, y, w, isMissing) => {
            doc.setFont("helvetica", "normal");
            doc.setFontSize(7);
            doc.setTextColor(100);
            doc.text(label, x, y);
            
            doc.setFont("helvetica", "bold");
            doc.setFontSize(9);
            if (isMissing) {
                doc.setTextColor(255, 0, 0);
                doc.setFillColor(255, 230, 230);
                doc.rect(x, y + 1, w, 6, 'F');
                doc.text("[ MISSING ]", x + 1, y + 5);
            } else {
                doc.setTextColor(0);
                doc.text(String(value || ""), x + 1, y + 5);
            }
            doc.line(x, y + 7, x + w, y + 7);
        };

        drawHeader();

        // 1. Filing Status
        let curY = 32;
        doc.setFontSize(8);
        doc.text("Filing Status: " + (taxData['Filing Status'] || "____"), 10, curY);
        curY += 8;

        // 2. Personal Info
        curY = drawSection("Personal Information", curY);
        curY += 5;
        drawField("First Name", taxData['First Name'], 10, curY, 60, !taxData['First Name']);
        drawField("Last Name", taxData['Last Name'], 75, curY, 60, !taxData['Last Name']);
        drawField("Social Security Number", taxData['SSN'], 140, curY, 55, !taxData['SSN']);
        
        curY += 12;
        drawField("Home Address", taxData['Street Address'], 10, curY, 110, !taxData['Street Address']);
        drawField("City", taxData['City'], 125, curY, 30, !taxData['City']);
        drawField("State", taxData['State'], 160, curY, 10, !taxData['State']);
        drawField("ZIP", taxData['ZIP Code'], 175, curY, 20, !taxData['ZIP Code']);

        // 3. Income Section (Simplified 1040 lines)
        curY += 15;
        curY = drawSection("Income", curY);
        curY += 5;

        const incomeLines = [
            ["1z", "Wages, salaries, tips", '1z: Wages, salaries, tips'],
            ["2b", "Taxable interest", '2b: Taxable Interest'],
            ["3b", "Ordinary dividends", '3b: Ordinary Dividends'],
            ["4b", "IRA distributions", '4b: IRA distributions'],
            ["5b", "Pensions and annuities", '5b: Pensions and annuities'],
            ["6b", "Social security benefits", '6b: Social security benefits'],
            ["7", "Capital gain or (loss)", '7: Capital gain or (loss)']
        ];

        incomeLines.forEach((line, i) => {
            const val = parseFloat(taxData[line[2]]) || 0;
            doc.setFontSize(8);
            doc.setTextColor(0);
            doc.text(line[0], 12, curY);
            doc.text(line[1], 18, curY);
            doc.text(val.toLocaleString('en-US', {style:'currency', currency:'USD'}), 190, curY, { align: 'right' });
            doc.line(18, curY + 1, 195, curY + 1);
            curY += 7;
        });

        // Calculations
        const totalIncome = incomeLines.reduce((acc, line) => acc + (parseFloat(taxData[line[2]]) || 0), 0);
        const adjustments = parseFloat(taxData['10: Adjustments to Income']) || 0;
        const agi = totalIncome - adjustments;
        const deduction = parseFloat(taxData['12: Standard/Itemized Deduction']) || 0;
        const taxableIncome = Math.max(0, agi - deduction);

        curY += 2;
        doc.setFont("helvetica", "bold");
        doc.text("9", 12, curY); doc.text("Total Income", 18, curY);
        doc.text(totalIncome.toLocaleString('en-US', {style:'currency', currency:'USD'}), 190, curY, { align: 'right' });
        
        curY += 7;
        doc.text("11", 12, curY); doc.text("Adjusted Gross Income (AGI)", 18, curY);
        doc.text(agi.toLocaleString('en-US', {style:'currency', currency:'USD'}), 190, curY, { align: 'right' });

        curY += 7;
        doc.text("15", 12, curY); doc.text("Taxable Income", 18, curY);
        doc.text(taxableIncome.toLocaleString('en-US', {style:'currency', currency:'USD'}), 190, curY, { align: 'right' });

        // 4. Payments
        curY += 10;
        curY = drawSection("Payments & Credits", curY);
        curY += 5;
        
        const withholding = parseFloat(taxData['25: Federal Income Tax Withheld']) || 0;
        doc.setFont("helvetica", "normal");
        doc.text("25", 12, curY); doc.text("Federal income tax withheld from Forms W-2 and 1099", 18, curY);
        doc.text(withholding.toLocaleString('en-US', {style:'currency', currency:'USD'}), 190, curY, { align: 'right' });

        // Signatures area
        curY = 240;
        doc.line(10, curY, 200, curY);
        doc.setFontSize(7);
        doc.text("Sign Here: _________________________________________________ Date: __________", 15, curY + 10);
        doc.text("Joint Return? Spouse Signature: ____________________________ Date: __________", 15, curY + 20);

        doc.save("Form_1040_Generated.pdf");
    });
});
