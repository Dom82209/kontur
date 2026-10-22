"""Генерирует синтетические fixtures для тестов извлечения текста (этап 7).

Содержимое придумано и не содержит личных данных. Запуск (нужны reportlab,
python-docx, openpyxl, python-pptx, pypdf, Pillow и шрифт DejaVu Sans):

    python3 generate.py

Метаданные авторов и дат задаются явно, чтобы файлы были воспроизводимыми
и не содержали имени пользователя машины, на которой их сгенерировали.
"""

import datetime
import io

FIXED = datetime.datetime(2026, 9, 1, 10, 0, 0)
AUTHOR = "Контур fixtures"


def pdfs():
    from reportlab.lib.pagesizes import A4
    from reportlab.pdfbase import pdfmetrics
    from reportlab.pdfbase.ttfonts import TTFont
    from reportlab.pdfgen import canvas
    from PIL import Image, ImageDraw

    pdfmetrics.registerFont(TTFont("DejaVu", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"))
    c = canvas.Canvas("lecture-cyrillic.pdf", pagesize=A4, invariant=1)
    c.setTitle("Лекция: описательная статистика")
    c.setAuthor(AUTHOR)
    c.setSubject("Синтетический учебный документ")
    c.setCreator("generate.py")
    for page in range(1, 4):
        c.setFont("DejaVu", 14)
        c.drawString(72, 780, f"Лекция {page}. Описательная статистика")
        c.setFont("DejaVu", 11)
        c.drawString(72, 750, "Среднее значение и медиана — меры центральной тенденции.")
        c.drawString(72, 735, f"Mean and median, page {page}.")
        c.showPage()
    c.save()

    image = Image.new("RGB", (600, 200), "white")
    ImageDraw.Draw(image).rectangle((20, 20, 580, 180), outline="black")
    buffer = io.BytesIO()
    image.save(buffer, format="PNG")
    buffer.seek(0)
    from reportlab.lib.utils import ImageReader

    c = canvas.Canvas("scanned.pdf", pagesize=A4, invariant=1)
    c.setAuthor(AUTHOR)
    c.drawImage(ImageReader(buffer), 72, 500, 400, 133)
    c.showPage()
    c.drawImage(ImageReader(buffer), 72, 500, 400, 133)
    c.showPage()
    c.save()

    from pypdf import PdfReader, PdfWriter

    writer = PdfWriter(clone_from=PdfReader("lecture-cyrillic.pdf"))
    writer.encrypt(user_password="secret-test-password", owner_password="owner-test", algorithm="AES-128")
    with open("encrypted.pdf", "wb") as handle:
        writer.write(handle)

    from PIL import ImageFont

    sample = Image.new("L", (520, 90), "white")
    font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 40)
    ImageDraw.Draw(sample).text((16, 18), "Kontur OCR test", fill="black", font=font)
    sample.save("ocr-sample.png", format="PNG", optimize=True)

    small = Image.new("RGB", (64, 32), "white")
    ImageDraw.Draw(small).text((4, 10), "Hi", fill="black")
    small.save("diagram.png", format="PNG")


def docx():
    from docx import Document

    document = Document()
    document.core_properties.author = AUTHOR
    document.core_properties.title = "Семинар 4"
    document.core_properties.created = FIXED
    document.core_properties.modified = FIXED
    document.core_properties.last_modified_by = AUTHOR
    document.add_heading("Семинар 4. Регрессия", level=1)
    document.add_paragraph("Цель семинара — построить линейную регрессию по данным продаж.")
    document.add_heading("Задачи", level=2)
    document.add_paragraph("Загрузить данные", style="List Bullet")
    document.add_paragraph("Построить диаграмму рассеяния", style="List Bullet")
    document.add_paragraph("Оценить коэффициенты", style="List Number")
    document.add_paragraph("Проверить остатки", style="List Number")
    table = document.add_table(rows=3, cols=2)
    for row, (left, right) in enumerate([("Месяц", "Продажи"), ("Январь", "120"), ("Февраль", "135")]):
        table.cell(row, 0).text = left
        table.cell(row, 1).text = right
    document.add_paragraph("Итог: коэффициент наклона положительный.")
    document.save("seminar.docx")


def xlsx():
    from openpyxl import Workbook

    workbook = Workbook()
    workbook.properties.creator = AUTHOR
    workbook.properties.lastModifiedBy = AUTHOR
    workbook.properties.created = FIXED
    workbook.properties.modified = FIXED
    workbook.properties.title = "Данные для ДЗ"
    sheet = workbook.active
    sheet.title = "Продажи"
    sheet.append(["Месяц", "Выручка", "Дата"])
    sheet.append(["Январь", 120, datetime.date(2026, 1, 31)])
    sheet.append(["Февраль", 135, datetime.date(2026, 2, 28)])
    sheet["A4"] = "Итого"
    sheet["B4"] = "=SUM(B2:B3)"
    for cell in ("C2", "C3"):
        sheet[cell].number_format = "DD.MM.YYYY"
    notes = workbook.create_sheet("Пояснения")
    notes["A1"] = "Выручка указана в тысячах рублей"
    hidden = workbook.create_sheet("Служебный")
    hidden["B2"] = 42
    hidden.sheet_state = "hidden"
    workbook.save("dataset.xlsx")


def pptx():
    from pptx import Presentation
    from pptx.util import Inches

    presentation = Presentation()
    presentation.core_properties.author = AUTHOR
    presentation.core_properties.title = "Неделя 5"
    presentation.core_properties.created = FIXED
    presentation.core_properties.modified = FIXED
    presentation.core_properties.last_modified_by = AUTHOR
    title = presentation.slides.add_slide(presentation.slide_layouts[0])
    title.shapes.title.text = "Когортный анализ"
    title.placeholders[1].text = "Неделя 5"
    content = presentation.slides.add_slide(presentation.slide_layouts[1])
    content.shapes.title.text = "Метрики удержания"
    body = content.placeholders[1].text_frame
    body.text = "Retention первого дня"
    body.add_paragraph().text = "Retention седьмого дня"
    content.notes_slide.notes_text_frame.text = "Напомнить про определение когорты."
    table_slide = presentation.slides.add_slide(presentation.slide_layouts[5])
    table_slide.shapes.title.text = "Таблица когорт"
    table = table_slide.shapes.add_table(2, 2, Inches(1), Inches(2), Inches(4), Inches(1)).table
    table.cell(0, 0).text = "Когорта"
    table.cell(0, 1).text = "D7"
    table.cell(1, 0).text = "Сентябрь"
    table.cell(1, 1).text = "31%"
    presentation.save("slides.pptx")


if __name__ == "__main__":
    pdfs()
    docx()
    xlsx()
    pptx()
