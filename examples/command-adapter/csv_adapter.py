"""명령 어댑터 예제(파이썬 3.8 이상, 표준 라이브러리만): 요청 JSON 을 받아 CSV(키 열 + 언어 열)를 쓰고 읽는다.

설정 예:
    내보내기 명령   python scripts/csv_adapter.py export {request}
    역가져오기 명령 python scripts/csv_adapter.py import {request}

요청의 options.key_header 로 키 열 머리글을 바꿀 수 있다(기본 Key).
실패하면 sys.exit("메시지") 처럼 0 이 아닌 코드로 끝내면 TMS 가 종료 코드와 오류 출력을 보여 준다.
"""

import csv
import json
import sys


def export(req):
    locales = req["locales"]
    key_header = req["options"].get("key_header", "Key")
    # utf-8-sig: 엑셀에서 열어도 한글이 깨지지 않게 BOM 을 붙인다
    with open(req["path"], "w", encoding="utf-8-sig", newline="") as f:
        w = csv.writer(f)
        w.writerow([key_header, *locales])
        for row in req["rows"]:
            w.writerow([row["external_key"], *(row["values"].get(l, "") for l in locales)])


def import_(req):
    locales = req["locales"]
    key_header = req["options"].get("key_header", "Key")
    rows = []
    with open(req["path"], encoding="utf-8-sig", newline="") as f:
        for rec in csv.DictReader(f):
            key = (rec.get(key_header) or "").strip()
            if key:
                rows.append({"external_key": key, "values": {l: rec.get(l) or "" for l in locales if l in rec}})
    with open(req["response_path"], "w", encoding="utf-8") as f:
        json.dump({"rows": rows}, f, ensure_ascii=False)


def main():
    # TMS 는 오류 출력을 UTF-8 로 읽는다. 한국어 Windows 의 기본(cp949) 대신 UTF-8 로 쓴다
    sys.stderr.reconfigure(encoding="utf-8")
    if len(sys.argv) != 3:
        sys.exit("사용: csv_adapter.py <export|import> <요청 파일>")
    action, request_path = sys.argv[1], sys.argv[2]
    with open(request_path, encoding="utf-8") as f:
        req = json.load(f)
    if action == "export":
        export(req)
    elif action == "import":
        import_(req)
    else:
        sys.exit(f"알 수 없는 동작: {action}")


if __name__ == "__main__":
    main()
