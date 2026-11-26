---
name: pdf2md
description: PDF를 로컬에서 Markdown으로 변환한 뒤 그 Markdown을 읽어 사용자의 요청(요약, 질문, 분석)을 처리한다. 스캔 페이지는 한국어 OCR로 읽는다. 사용자가 이 skill을 명시적으로 호출했을 때만 사용한다.
disable-model-invocation: true
---

# pdf2md

PDF를 직접 읽지 말고, `pdf2md` 실행 파일로 Markdown을 만든 뒤 그 Markdown을 읽는다. 목적은 토큰 절약이다.

네트워크는 1단계의 설치 스크립트가 처음 설치하거나 버전이 바뀌었을 때만 쓴다. 변환 자체는 전부 로컬에서 이루어진다.

## 경로

- 데이터 폴더: `${CLAUDE_PLUGIN_DATA}`. plugin을 업데이트해도 유지된다.
- 실행 파일: `${CLAUDE_PLUGIN_DATA}/bin/pdf2md`. Windows에서는 `${CLAUDE_PLUGIN_DATA}/bin/pdf2md.exe`다.
- OCR 모델: `${CLAUDE_PLUGIN_DATA}/bin/models/`. 실행 파일이 자동으로 찾으므로 따로 지정하지 않는다.
- 설치 스크립트: `${CLAUDE_SKILL_DIR}/scripts/install.sh`(Linux), `${CLAUDE_SKILL_DIR}/scripts/install.ps1`(Windows)

## 절차

1. **설치 확인**: 변환할 때마다 먼저 OS에 맞는 설치 스크립트를 실행한다. 이미 최신이면 스크립트는 아무것도 받지 않고 바로 끝난다. 경로는 따옴표로 감싼다.
   - Linux:
     ```sh
     sh "${CLAUDE_SKILL_DIR}/scripts/install.sh" "${CLAUDE_PLUGIN_DATA}"
     ```
   - Windows: Bash 도구(Git Bash)든 PowerShell 도구든 같은 명령을 쓴다.
     ```sh
     powershell -NoProfile -ExecutionPolicy Bypass -File "${CLAUDE_SKILL_DIR}/scripts/install.ps1" "${CLAUDE_PLUGIN_DATA}"
     ```
   - 종료 코드가 0이 아니면 아래 "실패 처리"를 따른다.
2. 사용자 요청에서 PDF 경로를 확인한다. 경로가 모호하면 사용자에게 묻는다.
3. 사용자가 특정 페이지만 언급했으면 `--pages`로 넘긴다(예: 3~5페이지 → `--pages 3-5`, 4페이지 → `--pages 4`). 언급이 없으면 옵션 없이 전체를 변환한다.
4. 실행한다. 경로는 따옴표로 감싼다.
   ```sh
   "${CLAUDE_PLUGIN_DATA}/bin/pdf2md" "<pdf 경로>" [--pages A-B]
   ```
   스캔 페이지는 페이지당 수 초가 걸린다. 표준 에러의 `OCR p.N`은 진행 상황이다.
5. 종료 코드가 0이면 PDF와 같은 디렉터리에 생긴 `<PDF 이름>.md`를 읽고 사용자의 요청을 처리한다.
   - 각 페이지는 `<!-- p.N -->`으로 시작한다. 파일이 크면 필요한 페이지 구간만 나눠 읽는다.
   - `[image p.N]`은 이미지가 있던 자리다. 이미지 내용은 알 수 없다.
6. 표준 출력의 OCR 보고를 확인한다.
   - `OCR pages: none`이면 OCR 관련 안내를 하지 않는다.
   - 그 외에는 보고 내용(페이지 목록, `(low)`, `(partial ...)` 표시 포함)을 대화창에 그대로 보여준다. 그리고 해당 페이지는 이미지에서 인식한 텍스트라 오류가 있을 수 있다고 알린다. `(partial ...)`이 붙은 페이지는 페이지 내용 일부만 읽었을 수 있다고 함께 알린다.
   - 이 경우에도 사용자에게 추가 확인을 묻지 않는다. 해당 페이지를 원본 PDF나 이미지로 다시 읽지 않는다. `.md` 내용만으로 작업을 계속한다.

## 하지 말 것

- PDF 파일을 직접 읽지 않는다(Read 도구 등으로 `.pdf`를 여는 것 포함). 설치나 변환에 실패했을 때도 마찬가지다.
- 변환이나 OCR을 위해 외부 서비스, 웹 API, 다른 OCR 도구를 쓰지 않는다. 설치 스크립트 말고는 아무것도 내려받거나 설치하지 않는다.

## 실패 처리

실패하면 원인과 해결 방법을 사용자에게 알리고 멈춘다. PDF 직접 읽기로 대체하지 않는다. 어떤 실패든 스크립트나 CLI의 표준 에러 메시지(`pdf2md install: ...`, `error: ...`)를 그대로 보여준다.

| 상황 | 사용자에게 알릴 내용 |
|---|---|
| 설치 스크립트 실패: `unsupported platform` | 이 OS는 아직 지원하지 않는다(지원: Linux x64, Windows x64) |
| 설치 스크립트 실패: `download failed` | 설치 파일을 내려받지 못했다. 네트워크 연결을 확인하라. 메시지의 주소를 함께 보여준다 |
| 설치 스크립트 실패: `checksum mismatch` | 내려받은 파일이 검증을 통과하지 못해 설치하지 않았다. 잠시 뒤 다시 시도하라 |
| 종료 코드 2 | 페이지 범위가 잘못되었거나 문서 페이지 수를 넘었다 |
| 종료 코드 3 | 입력 PDF를 열 수 없다. 손상되었거나 열람 암호가 걸려 있다 |
| 종료 코드 4 | 스캔 페이지가 있는데 OCR 모델이 없다. 설치 스크립트를 다시 실행하라 |
| 종료 코드 5 | OCR 실행에 실패했다 |
| 종료 코드 6 | PDF는 정상이지만 결과 `.md` 파일을 쓸 수 없다. 출력 위치의 쓰기 권한이나 같은 이름의 폴더를 확인하라 |
