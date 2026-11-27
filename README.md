# Pdf2md

PDF를 LLM이 적은 토큰으로 읽을 수 있는 Markdown으로 바꾸는 단일 실행 파일 CLI다. 디지털 PDF는 text layer를 추출하고, 스캔 페이지는 한국어 OCR로 읽는다. 변환은 전부 로컬에서 이루어지며, 실행 중에는 네트워크에 접근하지 않는다.

- 엔진: [pdf_oxide](https://github.com/yfedoseev/pdf_oxide) (`MIT OR Apache-2.0`)
- OCR runtime: 순수 Rust `tract` (ONNX Runtime 같은 공유 라이브러리가 필요 없음)
- OCR 모델: PaddleOCR PP-OCRv4 detection과 PP-OCRv5 한국어 recognition (Apache-2.0, `THIRD_PARTY_LICENSES` 참고)
- 지원 OS: Linux x86_64(glibc 2.35 이상), Windows x86_64

## 설치 (Claude Code plugin)

Claude Code에서 다음을 실행한다.

```
/plugin marketplace add dktmzdf/Pdf2md
/plugin install pdf2md@pdf2md
```

Claude Code 2.1.275 이상이면 한 줄로도 된다: `/plugin install pdf2md --marketplace dktmzdf/Pdf2md`

설치한 뒤 `/pdf2md`로 skill을 호출하면(예: `/pdf2md report.pdf 요약해줘`), 처음 한 번은 skill이 설치 스크립트를 실행한다. 스크립트는 OS에 맞는 바이너리를 GitHub Release에서, OCR 모델을 HuggingFace에서 받아 검증한 뒤 `${CLAUDE_PLUGIN_DATA}/bin/`(`~/.claude/plugins/data/<plugin>/bin/`)에 설치한다. 이후 변환은 네트워크 없이 이루어진다. plugin을 업데이트하면 다음 호출 때 새 버전 바이너리로 바뀐다. plugin을 삭제하면 이 폴더도 지워진다.

skill은 명시적으로 호출할 때만 동작한다. PDF를 언급하는 것만으로는 실행되지 않는다.

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
- 디지털 글자(text layer)는 서식 없는 평문으로 낸다. 제목(`#`), 굵게(`**`) 같은 서식 표시는 넣지 않는다. 같은 문단의 줄은 한 줄로 이어질 수 있다.
- 이미지는 추출하지 않는다. 페이지에 그려진 이미지마다 `[image p.N]` 한 줄을 남긴다. 스캔 페이지에서 OCR로 읽은 스캔 이미지는 그림이 아니므로 남기지 않는다.
- 스캔 페이지와, 텍스트와 이미지가 섞인 페이지는 다음과 같이 읽는다.
  - 머리글, 페이지 번호 같은 디지털 글자는 text layer에서 그대로 가져온다.
  - 페이지를 보이는 그대로 렌더링한 뒤 이미지 영역만 잘라 OCR한다. 그래서 여러 조각으로 나뉜 스캔도 한 번에 읽고, PDF에 적힌 페이지 회전(`/Rotate`)과 표시 영역(CropBox)도 반영된다.
  - 맞닿거나 겹친 이미지는 하나로 합친다. 합친 영역이 페이지 면적의 0.5% 미만이면(로고, 아이콘) OCR하지 않는다. 그래서 작은 타일 여러 개로 나뉜 스캔도 읽는다.
  - text layer가 없고 OCR할 이미지도 없는 페이지(글자를 도형으로 그린 PDF 등)는 페이지 전체를 OCR한다.
  - 회전 정보 없이 이미지 자체가 옆으로 누운 스캔은 바로 세우지 않는다.
- 표:
  - text layer에서 인식한 표는 행마다 한 줄로, 칸은 공백으로 구분해 출력한다(`Apple  3  1.20`). `|` 표나 HTML 표로 내지 않는다.
  - 스캔 페이지의 표는 구조를 복원하지 않고, 셀 텍스트를 읽기 순서대로 일반 텍스트로 출력한다.
- 머리글, 바닥글, 페이지 번호는 지우지 않는다.

### OCR 페이지 보고

변환이 끝나면 OCR로 읽은 페이지 목록을 표준 출력에 낸다. OCR 평균 신뢰도가 0.90 미만인 페이지에는 `(low)`가 붙는다. 페이지 렌더링에 실패해서 가장 큰 이미지 하나만 읽은 페이지에는 `(partial: only part of the page could be read)`가 붙는다. 다만 신뢰도는 실제 인식 오류를 잘 반영하지 못하므로, OCR 페이지는 모두 오류가 있을 수 있다고 보는 게 안전하다. 진행 상황(`OCR p.N`)은 표준 에러로 나간다.

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
| 3 | 입력 PDF를 열 수 없음 (손상, 열람 암호) |
| 4 | OCR이 필요한데 모델 파일이 없음 |
| 5 | OCR 실행 실패 |
| 6 | 출력 `.md` 파일을 쓸 수 없음 (권한, 같은 이름의 디렉터리 등) |

실패하면 `.md` 파일을 만들지 않는다.

## OCR 모델

스캔 페이지가 있을 때만 필요하다. 모델 디렉터리는 다음 순서로 찾는다.

1. `--models DIR`
2. 환경 변수 `PDF2MD_MODEL_DIR`
3. 실행 파일 옆의 `models/`

디렉터리에는 다음 세 파일이 있어야 한다. 문자 사전은 `inference.yml` 안의 `character_dict` 목록(11945자)을 바이너리가 직접 읽는다.

| 파일 | 내용 | 출처 (커밋 고정) | SHA256 |
|---|---|---|---|
| `det.onnx` | PP-OCRv4 text detection | `deepghs/paddleocr` @ `e7bd63d9c674c96bd4fdab435af0efb383919c18`, `det/ch_PP-OCRv4_det/model.onnx` | `30a86f5731181461d08021402766601e4302a9b9b9666be8aff402696339cdff` |
| `rec.onnx` | PP-OCRv5 Korean recognition | `PaddlePaddle/korean_PP-OCRv5_mobile_rec_onnx` @ `5c6f574b8e2230adf4287b33e736d71b9fabd28e`, `inference.onnx` | `92f0b7785e64fc9090106a241cf4c1eb97472824558272751b88a2a4476d3a08` |
| `inference.yml` | 위 rec 모델의 설정과 문자 사전 | 같은 저장소와 커밋, `inference.yml` | `f757fa1c40e99edcf27e9cce879b93eb2a51fa46f5ef39095689b8c37dd75998` |

plugin으로 설치하면 첫 실행 때 자동으로 받는다. 수동으로 배치하는 방법은 다음과 같다. 모델을 받을 때만 네트워크가 필요하다.

```sh
mkdir -p models && cd models
H=https://huggingface.co
curl -fL -o det.onnx $H/deepghs/paddleocr/resolve/e7bd63d9c674c96bd4fdab435af0efb383919c18/det/ch_PP-OCRv4_det/model.onnx
curl -fL -o rec.onnx $H/PaddlePaddle/korean_PP-OCRv5_mobile_rec_onnx/resolve/5c6f574b8e2230adf4287b33e736d71b9fabd28e/inference.onnx
curl -fL -o inference.yml $H/PaddlePaddle/korean_PP-OCRv5_mobile_rec_onnx/resolve/5c6f574b8e2230adf4287b33e736d71b9fabd28e/inference.yml
sha256sum det.onnx rec.onnx inference.yml   # 위 표의 SHA256과 같은지 확인
```

## 개발

```sh
cargo build --release
PDF2MD_MODEL_DIR=/path/to/models cargo test --release
```

- OCR이 필요한 테스트와 설치 스크립트 테스트(`tests/install.rs`)는 `PDF2MD_MODEL_DIR`이 없으면 실패한다. 설치 스크립트 테스트는 Release와 HuggingFace를 로컬 HTTP 서버로 흉내 낸다.
- 네트워크 차단 테스트는 Linux의 `unshare -rn`을 쓴다.

### 릴리스

버전은 다음 네 곳이 같아야 한다. 다르면 release CI가 실패한다.

- `Cargo.toml`
- `.claude-plugin/plugin.json`
- `skills/pdf2md/scripts/install.sh`의 `VERSION`
- `skills/pdf2md/scripts/install.ps1`의 `$Version`

네 곳을 맞춘 뒤 `vX.Y.Z` 태그를 push하면 GitHub Actions(`.github/workflows/release.yml`)가 다음을 차례로 진행한다.

1. Linux x64와 Windows x64에서 빌드하고 테스트한다.
2. prerelease로 Release를 만든다.
3. 실제 설치 스크립트로 설치해서 검증한다.
4. 검증을 통과하면 정식 Release로 바꾼다.

## 알려진 한계

- 스캔 페이지는 페이지당 수 초가 걸리고, 메모리를 최대 약 1.4GB까지 쓴다.
- 디지털 표의 글자가 칸이 붙은 한 줄(`Apple31.20Pear50.80`)로 한 번 더 나오는 경우가 있다.
- 단어 사이 공백을 글자 코드 없는 글리프로 넣은 PDF(Firefox 인쇄 PDF 등)는 단어가 붙어서 나온다.
- 페이지 트리의 `/Kids`가 간접 참조인 PDF(일부 calibre 전자책)는 페이지 번호가 어긋나고 렌더링에 실패한다. 이런 PDF는 `<!-- p.N -->`가 다른 페이지 내용을 담을 수 있고, 이미지 영역 OCR이 `partial`이 된다. pdf_oxide 0.3.78의 문제다.

## 라이선스

pdf2md는 [MIT](LICENSE-MIT) 또는 [Apache-2.0](LICENSE-APACHE) 중 원하는 쪽을 골라 쓸 수 있다(이중 라이선스). OCR 모델은 별도 저작물이며 Apache-2.0이다. 출처와 라이선스 전문은 `THIRD_PARTY_LICENSES`에 있다.
