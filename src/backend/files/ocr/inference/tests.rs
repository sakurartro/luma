use super::*;


// Живой прогон на реальных моделях/картинках:
//   OCR_DIR=/tmp/opencode/ocr OCR_IMG=/tmp/opencode/ocr/test.png cargo test -- --ignored ocr_recognize
#[test]
#[ignore]
fn ocr_recognize() {
    let dir = std::env::var("OCR_DIR").unwrap();
    let img = std::env::var("OCR_IMG").unwrap();
    let Some(mut models) = OcrModels::load_from(Path::new(&dir)) else {
        panic!("модели в {dir} не читаются");
    };
    let start = std::time::Instant::now();
    let text = models.recognize(Path::new(&img)).unwrap();
    println!("=== {img} === (за {:?})\n{text}", start.elapsed());
    assert!(!text.trim().is_empty(), "пустой текст");
}

// Отладка det: боксы и rec по каждому кропу.
#[test]
#[ignore]
fn ocr_debug_det() {
    let dir = std::env::var("OCR_DIR").unwrap();
    let img_path = std::env::var("OCR_IMG").unwrap();
    let Some(mut models) = OcrModels::load_from(Path::new(&dir)) else {
        panic!("модели в {dir} не читаются");
    };
    let (img, det_input, scale) = preprocess_det(Path::new(&img_path)).unwrap();
    println!("img {}x{}, scale {scale}", img.width(), img.height());
    let t0 = std::time::Instant::now();
    let (probs, dh, dw) = models.run_det(det_input).unwrap();
    println!("det out {dh}x{dw} за {:?}", t0.elapsed());
    let boxes = find_boxes(&probs, dh, dw);
    println!("боксов {}", boxes.len());
    let t1 = std::time::Instant::now();
    let mut n = 0;
    for (x0, y0, x1, y1) in boxes {
        n += 1;
        let tc = std::time::Instant::now();
        let crop_w = ((x1 - x0) / scale).round().max(1.0) as u32;
        let crop_h = ((y1 - y0) / scale).round().max(1.0) as u32;
        let cx0 = ((x0 / scale).round().max(0.0) as u32).min(img.width().saturating_sub(1));
        let cy0 = ((y0 / scale).round().max(0.0) as u32).min(img.height().saturating_sub(1));
        let crop_w = crop_w.min(img.width() - cx0);
        let crop_h = crop_h.min(img.height() - cy0);
        println!("box ({x0},{y0})-({x1},{y1}) -> crop {cx0},{cy0} {crop_w}x{crop_h} за {:?}", tc.elapsed());
        let crop = image::imageops::crop_imm(&img, cx0, cy0, crop_w, crop_h).to_image();
        println!(
            "  -> {:?} за {:?}",
            models.recognize_batch(&[crop]).map(|mut v| v.remove(0)),
            tc.elapsed()
        );
    }
    println!("rec {} кропов за {:?}", n, t1.elapsed());
}

// Отладка батча: форма выхода rec при N>1.
#[test]
#[ignore]
fn ocr_debug_batch() {
    let dir = std::env::var("OCR_DIR").unwrap();
    let img_path = std::env::var("OCR_IMG").unwrap();
    let Some(mut models) = OcrModels::load_from(Path::new(&dir)) else {
        panic!("модели в {dir} не читаются");
    };
    let img = image::open(&img_path).unwrap().to_rgb8();
    // Три одинаковых кропа, чтобы отличить раскладки по содержимому.
    let half = img.width() / 3;
    let crops: Vec<image::RgbImage> = (0..3)
        .map(|i| image::imageops::crop_imm(&img, i * half, 0, half, img.height()).to_image())
        .collect();
    let input = preprocess_rec_batch(&crops).unwrap();
    println!("input {:?}", input.dim());
    let outputs = models.rec.run(ort::inputs!["x" => Tensor::from_array(input).unwrap()]).unwrap();
    let (_, value) = outputs.iter().next().unwrap();
    let (shape, data) = value.try_extract_tensor::<f32>().unwrap();
    println!("out shape {shape:?}, len {}", data.len());
    let classes = shape[shape.len() - 1] as usize;
    let n = 3;
    let t = data.len() / (classes * n);
    // Печатаем декод в трёх гипотезах раскладки.
    let layouts: [(&str, &dyn Fn(usize, usize) -> usize); 3] = [
        ("[T,N,C]", &|i, r| (i * n + r) * classes),
        ("[N,T,C]", &|i, r| (r * t + i) * classes),
        ("[T,C,N]", &|i, r| i * classes + r),
    ];
    for (name, idx) in layouts {
        let mut texts = Vec::new();
        for row in 0..n {
            let mut text = String::new();
            let mut prev = 0usize;
            for i in 0..t {
                let base = idx(i, row);
                if base + classes > data.len() {
                    break;
                }
                let r = &data[base..base + classes];
                let best = r
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .unwrap()
                    .0;
                if best != 0 && best != prev {
                    text.push_str(models.dict.get(best - 1).map(|s| s.as_str()).unwrap_or(" "));
                }
                prev = best;
            }
            texts.push(text);
        }
        println!("{name}: {texts:?}");
    }
}

// Отладка rec без det: сырые argmax-классы по всей картинке.
#[test]
#[ignore]
fn ocr_debug_rec() {
    let dir = std::env::var("OCR_DIR").unwrap();
    let img_path = std::env::var("OCR_IMG").unwrap();
    let Some(mut models) = OcrModels::load_from(Path::new(&dir)) else {
        panic!("модели в {dir} не читаются");
    };
    let img = image::open(&img_path).unwrap().to_rgb8();
    let input = preprocess_rec_batch(&[img]).unwrap();
    let outputs = models.rec.run(ort::inputs!["x" => Tensor::from_array(input).unwrap()]).unwrap();
    let (_, value) = outputs.iter().next().unwrap();
    let (shape, data) = value.try_extract_tensor::<f32>().unwrap();
    let classes = shape[shape.len() - 1] as usize;
    let t = data.len() / classes;
    println!("shape {shape:?}, classes {classes}, dict len {}", models.dict.len());
    let mut ids = Vec::new();
    for i in 0..t {
        let row = &data[i * classes..(i + 1) * classes];
        let best = row
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap()
            .0;
        ids.push(best);
    }
    println!("argmax: {ids:?}");
    for off in [0usize, 1] {
        let s: String = ids
            .iter()
            .filter(|&&b| b != 0)
            .map(|&b| models.dict.get(b - 1 + off).map(|s| s.as_str()).unwrap_or("?").to_string())
            .collect();
        println!("off {off}: {s:?}");
    }
}
