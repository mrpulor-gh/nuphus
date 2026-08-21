# PaddleOCR ONNX 模型

Nuphus 使用 PaddleOCR PP-OCRv4 进行 OCR 识别。

## 所需文件

| 文件 | 用途 | 大小 |
|------|------|------|
| `ch_PP-OCRv4_det.onnx` | 文本检测模型 | ~4.6 MB |
| `ch_PP-OCRv4_rec.onnx` | 文本识别模型 | ~9.2 MB |
| `ch_PP-OCR_keys_v1.txt` | 字符字典（6623 类） | ~93 KB |

## 自动下载

`cargo build` 时通过 `src-tauri/build.rs` 自动从以下源下拉：
- 模型：`hf-mirror.com/SWHL/RapidOCR`（HuggingFace 镜像）
- 字典：`gitee.com/paddlepaddle/PaddleOCR`（GitHub 镜像）

若网络不可达，build 不会中断，会打印 warning 并提示手动放置路径。

## ONNX Runtime

运行 OCR 需要 `onnxruntime.dll`。build.rs 会自动将其从 `desktop/` 复制到 `target/debug/`。
