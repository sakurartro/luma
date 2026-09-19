#!/usr/bin/env python3
"""Reproducible INT8 SigLIP2 vs CLIP ViT-B/32 embedding benchmark.

The benchmark intentionally uses only dependencies already used by the project:
Pillow, NumPy, tokenizers, and ONNX Runtime. It creates a small deterministic
English image-text retrieval set in memory, L2-normalizes both modalities, and
uses their dot product as cosine similarity.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import platform
import statistics
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

import numpy as np
import onnxruntime as ort
from PIL import Image, ImageDraw
from tokenizers import Tokenizer


ROOT = Path(__file__).resolve().parents[1]
CANVAS_SIZE = 512


@dataclass(frozen=True)
class ModelSpec:
    name: str
    directory: Path
    text_model: Path
    vision_model: Path
    tokenizer: Path
    sequence_length: int
    image_size: int
    image_mean: tuple[float, float, float]
    image_std: tuple[float, float, float]
    resample: Image.Resampling
    text_output: str
    vision_output: str


def polygon_points(
    center: tuple[float, float], radius: float, count: int, rotation: float = -math.pi / 2
) -> list[tuple[float, float]]:
    cx, cy = center
    return [
        (
            cx + radius * math.cos(rotation + 2 * math.pi * i / count),
            cy + radius * math.sin(rotation + 2 * math.pi * i / count),
        )
        for i in range(count)
    ]


def star_points(
    center: tuple[float, float], outer: float, inner: float, count: int = 5
) -> list[tuple[float, float]]:
    points: list[tuple[float, float]] = []
    cx, cy = center
    for i in range(count * 2):
        angle = -math.pi / 2 + i * math.pi / count
        radius = outer if i % 2 == 0 else inner
        points.append((cx + radius * math.cos(angle), cy + radius * math.sin(angle)))
    return points


def base_image(color: str = "white") -> tuple[Image.Image, ImageDraw.ImageDraw]:
    image = Image.new("RGB", (CANVAS_SIZE, CANVAS_SIZE), color)
    return image, ImageDraw.Draw(image)


def red_circle() -> Image.Image:
    image, draw = base_image()
    draw.ellipse((106, 106, 406, 406), fill=(220, 35, 45), outline=(120, 10, 15), width=8)
    return image


def blue_square() -> Image.Image:
    image, draw = base_image()
    draw.rectangle((112, 112, 400, 400), fill=(35, 95, 220), outline=(10, 40, 120), width=8)
    return image


def green_triangle() -> Image.Image:
    image, draw = base_image()
    draw.polygon([(256, 78), (78, 408), (434, 408)], fill=(45, 165, 75), outline=(10, 90, 30), width=8)
    return image


def yellow_star() -> Image.Image:
    image, draw = base_image()
    draw.polygon(star_points((256, 260), 190, 78), fill=(250, 205, 25), outline=(150, 105, 0), width=8)
    return image


def horizontal_stripes() -> Image.Image:
    image, draw = base_image()
    for y in range(0, CANVAS_SIZE, 64):
        draw.rectangle((0, y, CANVAS_SIZE, y + 31), fill=(20, 20, 20))
    return image


def vertical_stripes() -> Image.Image:
    image, draw = base_image()
    for x in range(0, CANVAS_SIZE, 64):
        draw.rectangle((x, 0, x + 31, CANVAS_SIZE), fill=(20, 20, 20))
    return image


def checkerboard() -> Image.Image:
    image, draw = base_image()
    cell = 64
    for row in range(8):
        for col in range(8):
            if (row + col) % 2 == 0:
                draw.rectangle(
                    (col * cell, row * cell, (col + 1) * cell, (row + 1) * cell),
                    fill=(20, 20, 20),
                )
    return image


def rainbow_stripes() -> Image.Image:
    colors = [
        (220, 30, 45),
        (245, 125, 30),
        (245, 210, 35),
        (50, 165, 75),
        (35, 100, 220),
        (125, 55, 185),
    ]
    image, draw = base_image()
    stripe = CANVAS_SIZE / len(colors)
    for index, color in enumerate(colors):
        draw.rectangle((0, round(index * stripe), CANVAS_SIZE, round((index + 1) * stripe)), fill=color)
    return image


def right_arrow() -> Image.Image:
    image, draw = base_image()
    draw.polygon(
        [(70, 205), (305, 205), (305, 125), (450, 256), (305, 387), (305, 307), (70, 307)],
        fill=(25, 25, 25),
    )
    return image


def purple_heart() -> Image.Image:
    image, draw = base_image()
    draw.polygon([(256, 425), (95, 245), (417, 245)], fill=(135, 45, 170))
    draw.ellipse((83, 92, 270, 290), fill=(135, 45, 170))
    draw.ellipse((242, 92, 429, 290), fill=(135, 45, 170))
    return image


def snowman() -> Image.Image:
    image, draw = base_image((120, 190, 235))
    draw.rectangle((0, 390, CANVAS_SIZE, CANVAS_SIZE), fill=(235, 245, 250))
    draw.ellipse((155, 220, 357, 430), fill="white", outline=(170, 185, 195), width=5)
    draw.ellipse((185, 105, 327, 255), fill="white", outline=(170, 185, 195), width=5)
    draw.ellipse((220, 150, 234, 164), fill="black")
    draw.ellipse((278, 150, 292, 164), fill="black")
    draw.polygon([(256, 175), (256, 192), (305, 186)], fill=(235, 115, 20))
    return image


def tree_scene() -> Image.Image:
    image, draw = base_image((115, 185, 235))
    draw.rectangle((0, 370, CANVAS_SIZE, CANVAS_SIZE), fill=(100, 180, 75))
    draw.rectangle((225, 250, 287, 414), fill=(115, 70, 35))
    draw.ellipse((115, 80, 397, 315), fill=(40, 135, 55), outline=(15, 85, 30), width=6)
    return image


DATASET: tuple[tuple[str, Callable[[], Image.Image]], ...] = (
    ("a centered red circle on a white background", red_circle),
    ("a centered blue square on a white background", blue_square),
    ("a centered green triangle on a white background", green_triangle),
    ("a yellow five pointed star on a white background", yellow_star),
    ("black horizontal stripes on a white background", horizontal_stripes),
    ("black vertical stripes on a white background", vertical_stripes),
    ("a black and white checkerboard pattern", checkerboard),
    ("horizontal stripes in rainbow colors", rainbow_stripes),
    ("a black arrow pointing to the right", right_arrow),
    ("a purple heart on a white background", purple_heart),
    ("a snowman standing in the snow under a blue sky", snowman),
    ("a green tree under a blue sky", tree_scene),
)


def l2_normalize(vectors: np.ndarray) -> np.ndarray:
    vectors = np.asarray(vectors, dtype=np.float32)
    norms = np.linalg.norm(vectors, axis=-1, keepdims=True)
    if not np.all(np.isfinite(vectors)) or np.any(norms <= 1e-12):
        raise ValueError("Model returned non-finite or zero-length embeddings")
    return vectors / norms


def preprocess_images(images: list[Image.Image], spec: ModelSpec) -> np.ndarray:
    processed = []
    for image in images:
        image = image.convert("RGB").resize((spec.image_size, spec.image_size), spec.resample)
        array = np.asarray(image, dtype=np.float32) / 255.0
        array = (array - np.asarray(spec.image_mean, dtype=np.float32)) / np.asarray(
            spec.image_std, dtype=np.float32
        )
        processed.append(np.transpose(array, (2, 0, 1)))
    return np.stack(processed).astype(np.float32, copy=False)


def tokenize(texts: list[str], spec: ModelSpec) -> np.ndarray:
    tokenizer = Tokenizer.from_file(str(spec.tokenizer))
    tokenizer.enable_truncation(max_length=spec.sequence_length)
    if tokenizer.padding is None or tokenizer.padding.get("length") != spec.sequence_length:
        if spec.name.startswith("CLIP"):
            tokenizer.enable_padding(
                length=spec.sequence_length,
                pad_id=49407,
                pad_token="<|endoftext|>",
            )
        else:
            tokenizer.enable_padding(length=spec.sequence_length, pad_id=0, pad_token="<pad>")
    encodings = tokenizer.encode_batch(texts)
    input_ids = np.asarray([encoding.ids for encoding in encodings], dtype=np.int64)
    if input_ids.shape != (len(texts), spec.sequence_length):
        raise ValueError(f"Unexpected token matrix shape for {spec.name}: {input_ids.shape}")
    return input_ids


def median_runtime_ms(run: Callable[[], np.ndarray], warmup: int, repeats: int) -> tuple[np.ndarray, float]:
    output = run()
    for _ in range(max(0, warmup - 1)):
        output = run()
    timings = []
    for _ in range(repeats):
        started = time.perf_counter()
        output = run()
        timings.append((time.perf_counter() - started) * 1000.0)
    return output, statistics.median(timings)


def retrieval_metrics(similarities: np.ndarray) -> dict[str, float | int]:
    count = similarities.shape[0]
    expected = np.arange(count)
    diagonal = np.diag(similarities)
    off_diagonal = similarities[~np.eye(count, dtype=bool)]
    image_ranks = np.asarray(
        [1 + np.sum(similarities[row] > similarities[row, row]) for row in range(count)]
    )
    text_ranks = np.asarray(
        [1 + np.sum(similarities[:, col] > similarities[col, col]) for col in range(count)]
    )
    pairwise_image = np.mean(
        [
            similarities[row, row] > similarities[row, col]
            for row in range(count)
            for col in range(count)
            if row != col
        ]
    )
    pairwise_text = np.mean(
        [
            similarities[col, col] > similarities[row, col]
            for col in range(count)
            for row in range(count)
            if row != col
        ]
    )
    return {
        "pair_count": count,
        "matched_cosine_mean": float(np.mean(diagonal)),
        "matched_cosine_std": float(np.std(diagonal)),
        "mismatched_cosine_mean": float(np.mean(off_diagonal)),
        "mismatched_cosine_std": float(np.std(off_diagonal)),
        "matched_minus_mismatched": float(np.mean(diagonal) - np.mean(off_diagonal)),
        "image_to_text_top1": float(np.mean(np.argmax(similarities, axis=1) == expected)),
        "text_to_image_top1": float(np.mean(np.argmax(similarities, axis=0) == expected)),
        "image_to_text_mrr": float(np.mean(1.0 / image_ranks)),
        "text_to_image_mrr": float(np.mean(1.0 / text_ranks)),
        "image_pairwise_ranking_accuracy": float(pairwise_image),
        "text_pairwise_ranking_accuracy": float(pairwise_text),
    }


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def benchmark_model(
    spec: ModelSpec, images: list[Image.Image], texts: list[str], warmup: int, repeats: int
) -> dict[str, object]:
    for path in (spec.text_model, spec.vision_model, spec.tokenizer):
        if not path.is_file():
            raise FileNotFoundError(f"Required artifact does not exist: {path}")

    session_options = ort.SessionOptions()
    session_options.intra_op_num_threads = 1
    session_options.inter_op_num_threads = 1
    session_options.execution_mode = ort.ExecutionMode.ORT_SEQUENTIAL

    load_started = time.perf_counter()
    text_session = ort.InferenceSession(
        str(spec.text_model), sess_options=session_options, providers=["CPUExecutionProvider"]
    )
    vision_session = ort.InferenceSession(
        str(spec.vision_model), sess_options=session_options, providers=["CPUExecutionProvider"]
    )
    load_ms = (time.perf_counter() - load_started) * 1000.0

    text_outputs = {item.name for item in text_session.get_outputs()}
    vision_outputs = {item.name for item in vision_session.get_outputs()}
    if spec.text_output not in text_outputs or spec.vision_output not in vision_outputs:
        raise ValueError(
            f"Expected outputs missing for {spec.name}: "
            f"text={sorted(text_outputs)}, vision={sorted(vision_outputs)}"
        )

    input_ids = tokenize(texts, spec)
    pixel_values = preprocess_images(images, spec)
    text_input_name = text_session.get_inputs()[0].name
    vision_input_name = vision_session.get_inputs()[0].name

    text_raw, text_ms = median_runtime_ms(
        lambda: text_session.run([spec.text_output], {text_input_name: input_ids})[0],
        warmup,
        repeats,
    )
    vision_raw, vision_ms = median_runtime_ms(
        lambda: vision_session.run([spec.vision_output], {vision_input_name: pixel_values})[0],
        warmup,
        repeats,
    )
    text_embeddings = l2_normalize(text_raw)
    image_embeddings = l2_normalize(vision_raw)
    if text_embeddings.shape != image_embeddings.shape:
        raise ValueError(
            f"Text/image embedding shapes differ for {spec.name}: "
            f"{text_embeddings.shape} vs {image_embeddings.shape}"
        )
    similarities = image_embeddings @ text_embeddings.T
    metrics = retrieval_metrics(similarities)
    predictions = []
    for index, caption in enumerate(texts):
        predicted = int(np.argmax(similarities[index]))
        predictions.append(
            {
                "image_index": index,
                "expected_caption": caption,
                "predicted_caption": texts[predicted],
                "matched_cosine": float(similarities[index, index]),
                "best_cosine": float(similarities[index, predicted]),
                "correct": predicted == index,
            }
        )

    model_bytes = spec.text_model.stat().st_size + spec.vision_model.stat().st_size
    return {
        "name": spec.name,
        "embedding_dimension": int(text_embeddings.shape[1]),
        "onnx_model_bytes": model_bytes,
        "onnx_model_mib": model_bytes / (1024 * 1024),
        "session_load_ms": load_ms,
        "text_batch_median_ms": text_ms,
        "vision_batch_median_ms": vision_ms,
        "text_median_ms_per_item": text_ms / len(texts),
        "vision_median_ms_per_item": vision_ms / len(images),
        "text_model_sha256": sha256(spec.text_model),
        "vision_model_sha256": sha256(spec.vision_model),
        "metrics": metrics,
        "predictions": predictions,
        "similarity_matrix": similarities.tolist(),
    }


def model_specs() -> list[ModelSpec]:
    return [
        ModelSpec(
            name="SigLIP2 INT8",
            directory=ROOT / "models" / "siglip",
            text_model=ROOT / "models" / "siglip" / "text_model_int8.onnx",
            vision_model=ROOT / "models" / "siglip" / "vision_model_int8.onnx",
            tokenizer=ROOT / "models" / "siglip" / "tokenizer.json",
            sequence_length=64,
            image_size=224,
            image_mean=(0.5, 0.5, 0.5),
            image_std=(0.5, 0.5, 0.5),
            resample=Image.Resampling.BILINEAR,
            text_output="pooler_output",
            vision_output="pooler_output",
        ),
        ModelSpec(
            name="CLIP ViT-B/32 INT8",
            directory=ROOT / "models" / "clip",
            text_model=ROOT / "models" / "clip" / "onnx" / "text_model_int8.onnx",
            vision_model=ROOT / "models" / "clip" / "onnx" / "vision_model_int8.onnx",
            tokenizer=ROOT / "models" / "clip" / "tokenizer.json",
            sequence_length=77,
            image_size=224,
            image_mean=(0.48145466, 0.4578275, 0.40821073),
            image_std=(0.26862954, 0.26130258, 0.27577711),
            resample=Image.Resampling.BICUBIC,
            text_output="text_embeds",
            vision_output="image_embeds",
        ),
    ]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--warmup", type=int, default=2, help="Warm-up inference batches per encoder")
    parser.add_argument("--repeats", type=int, default=7, help="Timed inference batches per encoder")
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / "models" / "model_comparison_results.json",
        help="JSON output path",
    )
    args = parser.parse_args()
    if args.warmup < 1 or args.repeats < 1:
        parser.error("--warmup and --repeats must both be positive")

    texts = [caption for caption, _ in DATASET]
    images = [factory() for _, factory in DATASET]
    results = {
        "benchmark": {
            "description": "Deterministic synthetic English image-text retrieval sanity benchmark",
            "similarity": "dot product after independent L2 normalization (cosine similarity)",
            "pair_count": len(DATASET),
            "captions": texts,
            "warmup_batches": args.warmup,
            "timed_batches": args.repeats,
            "threads_per_session": 1,
        },
        "environment": {
            "python": sys.version.split()[0],
            "platform": platform.platform(),
            "processor": platform.processor(),
            "onnxruntime": ort.__version__,
            "numpy": np.__version__,
        },
        "models": [],
    }
    for spec in model_specs():
        print(f"Benchmarking {spec.name}...", flush=True)
        results["models"].append(benchmark_model(spec, images, texts, args.warmup, args.repeats))

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(results, indent=2) + "\n", encoding="utf-8")
    print(f"Wrote {args.output}")
    for result in results["models"]:
        metrics = result["metrics"]
        print(
            f"{result['name']}: I2T top-1={metrics['image_to_text_top1']:.1%}, "
            f"T2I top-1={metrics['text_to_image_top1']:.1%}, "
            f"match gap={metrics['matched_minus_mismatched']:.4f}, "
            f"text={result['text_median_ms_per_item']:.2f} ms/item, "
            f"vision={result['vision_median_ms_per_item']:.2f} ms/item"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
