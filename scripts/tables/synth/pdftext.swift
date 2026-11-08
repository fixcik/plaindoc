import Foundation
import PDFKit
// usage: pdftext OUTDIR a.pdf b.pdf ...  -> OUTDIR/<stem>.json: {"pages":[{"w":..,"h":..,"text":"...","tok":[[t,x,ytop],...]}]}
// Независимый от pdf_oxide текстовый слой (PDFKit). ytop: расстояние от верха страницы до верха первого символа токена.
let a = CommandLine.arguments
for f in a[2...] {
    let stem = URL(fileURLWithPath: f).deletingPathExtension().lastPathComponent
    var pages: [[String: Any]] = []
    if let doc = PDFDocument(url: URL(fileURLWithPath: f)) {
        for i in 0..<doc.pageCount {
            guard let page = doc.page(at: i) else { continue }
            let box = page.bounds(for: .mediaBox)
            let s = page.string ?? ""
            let ns = s as NSString
            var toks: [[Any]] = []
            var start = -1
            let n = ns.length
            func flush(_ end: Int) {
                if start >= 0 {
                    let t = ns.substring(with: NSRange(location: start, length: end - start))
                    var bx = -1.0, by = -1.0
                    if let sel = page.selection(for: NSRange(location: start, length: end - start)) {
                        let b = sel.bounds(for: page); bx = Double(b.minX); by = Double(box.height - b.maxY)
                    }
                    toks.append([t, bx, by])
                    start = -1
                }
            }
            for k in 0..<n {
                let c = ns.character(at: k)
                let isSpace = c == 32 || c == 9 || c == 10 || c == 13 || c == 0xA0 || c == 0x202F || c == 0x2009 || c == 0x2007 || c == 0x200B
                if isSpace { flush(k) } else if start < 0 { start = k }
            }
            flush(n)
            pages.append(["w": Double(box.width), "h": Double(box.height), "text": s, "tok": toks])
        }
    }
    let data = try! JSONSerialization.data(withJSONObject: ["pages": pages], options: [])
    try! data.write(to: URL(fileURLWithPath: "\(a[1])/\(stem).json"))
}
