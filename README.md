# Pdf2md

PDF를 LLM이 적은 토큰으로 읽을 수 있는 Markdown으로 바꾸는 단일 실행 파일 CLI다. 디지털 PDF는 text layer를 추출하고, 스캔 페이지는 한국어 OCR로 읽는다. 변환은 전부 로컬에서 이루어지며, 실행 중에는 네트워크에 접근하지 않는다.

- 엔진: [pdf_oxide](https://github.com/yfedoseev/pdf_oxide) (`MIT OR Apache-2.0`)
- OCR runtime: 순수 Rust `tract` (ONNX Runtime 같은 공유 라이브러리가 필요 없음)
- OCR 모델: PaddleOCR PP-OCRv4 detection과 PP-OCRv5 한국어 recognition (Apache-2.0, `THIRD_PARTY_LICENSES` 참고)

## 사용법

```sh
pdf2md <input.pdf> [--pages N|A-B] [--models DIR]
```

- 출력: 입력 PDF와 같은 디렉터리에 확장자만 `.md`로 바꾼 파일을 만든다. 같은 이름의 파일이 있으면 확인 없이 덮어쓴다.
- `--pages`: 변환할 페이지 범위다. 1부터 센다. 예: `--pages 3-5`, `--pages 4`. 생략하면 전체 페이지를 변환한다.
- `--models`: OCR 모델 디렉터리다. 아래 [OCR 모델](#ocr-모델) 참고.

```sh
pdf2md report.pdf               # report.md 생성
pdf2md report.pdf --pages 3-5   # 3~5페이지만 변환
```

### 출력 형식

- 각 페이지 앞에 `<!-- p.N -->` 마커를 넣는다.
- 이미지는 추출하지 않는다. 이미지마다 `[image p.N]` 한 줄만 남긴다.
- 표:
  - text layer에서 인식한 표는 GFM 표로 출력한다.
  - 스캔 페이지의 표는 구조를 복원하지 않고, 셀 텍스트를 읽기 순서대로 일반 텍스트로 출력한다.
- 머리글, 바닥글, 페이지 번호는 지우지 않는다.

### OCR 페이지 보고

변환이 끝나면 OCR로 읽은 페이지 목록을 표준 출력에 낸다. OCR 평균 신뢰도가 0.90 미만인 페이지에는 `(low)`가 붙는다. 다만 신뢰도는 실제 인식 오류를 잘 반영하지 못하므로, OCR 페이지는 모두 오류가 있을 수 있다고 보는 게 안전하다. 진행 상황(`OCR p.N`)은 표준 에러로 나간다.

```text
wrote /docs/lecture.md
OCR pages: 2 (text was recognized from images and may contain errors)
- p.12 confidence 0.99
- p.13 confidence 0.99
```

OCR한 페이지가 없으면 다음 한 줄만 낸다.

```text
OCR pages: none
```

### 종료 코드

| 코드 | 의미 |
|---|---|
| 0 | 성공 |
| 2 | 잘못된 인자 또는 페이지 범위 (예: 문서 페이지 수 초과) |
| 3 | PDF를 열 수 없음 (손상, 열람 암호) |
| 4 | OCR이 필요한데 모델 파일이 없음 |
| 5 | OCR 실행 실패 |

실패하면 `.md` 파일을 만들지 않는다.

## OCR 모델

스캔 페이지가 있을 때만 필요하다. 모델 디렉터리는 다음 순서로 찾는다.

1. `--models DIR`
2. 환경 변수 `PDF2MD_MODEL_DIR`
3. 실행 파일 옆의 `models/`

디렉터리에는 다음 세 파일이 있어야 한다.

| 파일 | 내용 | 출처 |
|---|---|---|
| `det.onnx` | PP-OCRv4 text detection | https://huggingface.co/deepghs/paddleocr `det/ch_PP-OCRv4_det/model.onnx` |
| `rec.onnx` | PP-OCRv5 Korean recognition | https://huggingface.co/PaddlePaddle/korean_PP-OCRv5_mobile_rec_onnx `inference.onnx` |
| `dict.txt` | 위 rec 모델의 문자 사전 (한 줄에 한 글자) | 같은 repo `inference.yml`의 `character_dict` |

수동으로 배치하는 방법은 다음과 같다. 모델을 받을 때만 네트워크가 필요하다.

```sh
mkdir -p models && cd models
curl -fL -o det.onnx https://huggingface.co/deepghs/paddleocr/resolve/main/det/ch_PP-OCRv4_det/model.onnx
curl -fL -o rec.onnx https://huggingface.co/PaddlePaddle/korean_PP-OCRv5_mobile_rec_onnx/resolve/main/inference.onnx
curl -fL -o inference.yml https://huggingface.co/PaddlePaddle/korean_PP-OCRv5_mobile_rec_onnx/resolve/main/inference.yml
python3 - <<'PY'
lines = open("inference.yml", encoding="utf-8").read().split("\n")
i = lines.index("  character_dict:") + 1
chars = []
while i < len(lines) and lines[i].startswith("  - "):
    v = lines[i][4:]
    if len(v) >= 2 and v[0] == v[-1] == "'":
        v = v[1:-1].replace("''", "'")
    elif len(v) >= 2 and v[0] == v[-1] == '"':
        v = bytes(v[1:-1], "utf-8").decode("unicode_escape")
    chars.append(v)
    i += 1
open("dict.txt", "w", encoding="utf-8").write("\n".join(chars))
print(len(chars), "chars")
PY
rm inference.yml
```

`dict.txt`는 11945자여야 한다.

## 개발

```sh
cargo build --release
PDF2MD_MODEL_DIR=/path/to/models cargo test --release
```

- OCR이 필요한 테스트는 `PDF2MD_MODEL_DIR`이 없으면 실패한다.
- 네트워크 차단 테스트는 Linux의 `unshare -rn`을 쓴다.

## 알려진 한계

- 스캔 페이지는 페이지당 수 초가 걸리고, 메모리를 최대 약 1.4GB까지 쓴다.
- pdf_oxide의 native 추출에서 단어 사이 공백이 빠지는 경우가 있다.
- 디지털 표를 GFM 표로 낼 때 같은 내용이 표 뒤에 일반 텍스트로 한 번 더 나오는 경우가 있다.
- 텍스트와 이미지가 섞인 페이지는 가장 큰 이미지 하나만 OCR한다.

## 라이선스

OCR 모델의 출처와 라이선스는 `THIRD_PARTY_LICENSES`에 있다.
