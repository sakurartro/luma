// Постпроцессинг: prob map det -> боксы, выход rec -> текст (CTC-декод).
// Чистые функции над срезами, без ort и без картинок.

// Порог бинаризации prob map.
const DET_THRESHOLD: f32 = 0.3;
// DBNet предсказывает сжатые внутрь текст-регионы; postprocess обязан
// расширить их обратно (как ClipperOffset в стандартном пайплайне).
const DET_UNCLIP: f32 = 1.5;

// Prob map [H*W] -> боксы (x0,y0,x1,y1) в координатах det-входа, отсортированные
// сверху вниз, слева направо. Без OpenCV: связные компоненты + bbox.
// shortcut: повернутый текст детектится криво (bbox по осям), когда пригодится —
// min-area rect по контуру.
pub(super) fn find_boxes(probs: &[f32], h: usize, w: usize) -> Vec<(f32, f32, f32, f32)> {
    if h == 0 || w == 0 || probs.len() != h * w {
        return Vec::new();
    }
    let mask: Vec<bool> = probs.iter().map(|&p| p >= DET_THRESHOLD).collect();
    let mut visited = vec![false; mask.len()];
    let mut boxes = Vec::new();
    for start in 0..mask.len() {
        if !mask[start] || visited[start] {
            continue;
        }
        // Flood fill одной компоненты.
        let mut stack = vec![start];
        visited[start] = true;
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (usize::MAX, usize::MAX, 0, 0);
        let mut area = 0usize;
        while let Some(idx) = stack.pop() {
            let (x, y) = (idx % w, idx / w);
            area += 1;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
            for (nx, ny) in [
                (x.wrapping_sub(1), y),
                (x + 1, y),
                (x, y.wrapping_sub(1)),
                (x, y + 1),
            ] {
                if nx < w && ny < h {
                    let n = ny * w + nx;
                    if mask[n] && !visited[n] {
                        visited[n] = true;
                        stack.push(n);
                    }
                }
            }
        }
        // Мусор: одиночные пиксели и шумы.
        if area < 20 {
            continue;
        }
        // Unclip: расширяем сжатый DB-бокс. distance = area*ratio/периметр,
        // как в стандартном DBPostProcess (аппроксимация для прямых боксов).
        let bw = (max_x - min_x + 1) as f32;
        let bh = (max_y - min_y + 1) as f32;
        let distance = (area as f32 * DET_UNCLIP) / (2.0 * (bw + bh)).max(1.0);
        let x0 = (min_x as f32 - distance).max(0.0);
        let y0 = (min_y as f32 - distance).max(0.0);
        let x1 = (max_x as f32 + distance).min(w as f32 - 1.0);
        let y1 = (max_y as f32 + distance).min(h as f32 - 1.0);
        boxes.push((x0, y0, x1, y1));
    }
    boxes.sort_by(|a, b| (a.1, a.0).partial_cmp(&(b.1, b.0)).unwrap_or(std::cmp::Ordering::Equal));
    boxes
}

// Выход rec (плоско) -> N строк. CTC: схлопываем повторы подряд,
// класс 0 — blank; 1..=dict.len() — словарь; хвост за словарём (97-й класс
// при 95 строках словаря) — пробел.
// Раскладку берём из формы: экспорт PaddleOCR — [N,T,C], но бывает и [T,N,C];
// при N == T они неразличимы, тогда [N,T,C] как у PaddleOCR.
pub(super) fn ctc_decode(data: &[f32], shape: &[i64], n: usize, dict: &[String]) -> Vec<String> {
    let classes = shape.last().copied().unwrap_or(0).max(1) as usize;
    let t = data.len() / (classes * n.max(1));
    let batch_first = shape.first().is_some_and(|&d| d as usize == n);
    let mut texts = Vec::with_capacity(n);
    for row in 0..n {
        let mut text = String::new();
        let mut prev = 0usize;
        for i in 0..t {
            let base = if batch_first { (row * t + i) * classes } else { (i * n + row) * classes };
            let r = &data[base..base + classes];
            let (best, _) = r
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
                .unwrap_or((0, &0.0));
            if best != 0 && best != prev {
                match dict.get(best - 1) {
                    Some(ch) => text.push_str(ch),
                    None => text.push(' '),
                }
            }
            prev = best;
        }
        texts.push(text);
    }
    texts
}

#[cfg(test)]
mod tests {
    use super::*;

    // One-hot logits for the class sequences of each row, in the given layout.
    fn logits(rows: &[[usize; 3]], classes: usize, batch_first: bool) -> Vec<f32> {
        let (n, t) = (rows.len(), 3);
        let mut data = vec![0.0; n * t * classes];
        for (row, seq) in rows.iter().enumerate() {
            for (i, &class) in seq.iter().enumerate() {
                let base = if batch_first { (row * t + i) * classes } else { (i * n + row) * classes };
                data[base + class] = 1.0;
            }
        }
        data
    }

    #[test]
    fn ctc_decode_reads_layout_from_shape() {
        let dict: Vec<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
        // classes: 0 blank, 1 a, 2 b, 3 space; repeats collapse, blank splits
        let rows = [[1, 1, 2], [2, 0, 2]];
        let want = ["ab", "bb"];
        assert_eq!(ctc_decode(&logits(&rows, 4, true), &[2, 3, 4], 2, &dict), want);
        assert_eq!(ctc_decode(&logits(&rows, 4, false), &[3, 2, 4], 2, &dict), want);
        assert_eq!(ctc_decode(&logits(&[[3, 1, 0]], 4, true), &[1, 3, 4], 1, &dict), [" a"]);
    }
}
