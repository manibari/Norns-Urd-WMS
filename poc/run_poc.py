#!/usr/bin/env python3
"""Run the recognition PoC over an annotated photo set.

Two-phase by design. Recognition costs money and is fixed once recorded;
matching thresholds are free to vary. So:

    ./run_poc.py --annotations a.csv --images photos/ --record run1.json
    ./run_poc.py --annotations a.csv --replay run1.json --sweep

The second command re-scores the same recognitions under every threshold
combination without calling the API again — that is how the false-hit vs
defer-rate tradeoff curve gets drawn (PoC spec section 6).
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import sys
from dataclasses import asdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "core"))

from urdwms_core.evaluate import (  # noqa: E402
    Evaluation, Outcome, Sample, evaluate, false_hit_upper_bound, summarize,
)
from urdwms_core.matching import Candidate  # noqa: E402
from urdwms_core.recognition import (  # noqa: E402
    ClaudeProvider, GeminiProvider, Recognition, ReplayProvider,
)

PASS_CRITERIA = {"false_hit_rate": 0.005, "hit_rate": 0.80, "defer_rate": 0.20}


def load_samples(path: Path) -> list[Sample]:
    samples: list[Sample] = []
    with path.open(newline="", encoding="utf-8") as handle:
        for row in csv.DictReader(handle):
            raw_lots = [d.strip() for d in (row.get("candidate_lots") or "").split("|") if d.strip()]
            samples.append(Sample(
                photo_id=row["photo_id"].strip(),
                item_code_truth=(row.get("item_code") or "").strip(),
                receipt_date_truth=(row.get("receipt_date_truth") or "").strip() or None,
                manufacture_date_truth=(row.get("manufacture_date_truth") or "").strip() or None,
                stratum=(row.get("stratum") or "unspecified").strip(),
                candidates=[Candidate(f"L{i}", d) for i, d in enumerate(raw_lots, start=1)],
            ))
    return samples


def recognize_all(samples: list[Sample], images: Path, provider) -> dict[str, Recognition]:
    results: dict[str, Recognition] = {}
    for i, sample in enumerate(samples, start=1):
        path = images / sample.photo_id
        if not path.exists():
            results[sample.photo_id] = Recognition(error=f"missing_file:{path}")
            print(f"  [{i}/{len(samples)}] {sample.photo_id}: MISSING", file=sys.stderr)
            continue
        reading = provider.recognize(path)
        results[sample.photo_id] = reading
        status = reading.error or f"receipt={reading.receipt_date!r} conf={reading.receipt_date_confidence:.2f}"
        print(f"  [{i}/{len(samples)}] {sample.photo_id}: {status}", file=sys.stderr)
    return results


def score(samples: list[Sample], readings: dict[str, Recognition], **thresholds) -> list[Evaluation]:
    return [
        evaluate(s, readings.get(s.photo_id, Recognition(error="no_recording")), **thresholds)
        for s in samples
    ]


def _pct(value: float | None) -> str:
    return "—" if value is None else f"{value * 100:.1f}%"


def _rate_row(label: str, rates: dict) -> str:
    return (f"| {label} | {rates['n']} | {_pct(rates['hit_rate'])} | "
            f"{_pct(rates['false_hit_rate'])} | {_pct(rates['defer_rate'])} |")


def render_report(summary: dict, thresholds: dict, evaluations: list[Evaluation], model: str) -> str:
    overall = summary["overall"]
    lines = [
        "# 辨識 PoC 結果",
        "",
        f"模型 `{model}`｜有章樣本 {overall['n']} 張（三率分母）｜"
        f"門檻 conf≥{thresholds['confidence_threshold']} "
        f"dist≤{thresholds['max_distance']} margin≥{thresholds['min_margin']}",
        "",
        "## 總計",
        "",
        "| 指標 | 值 | 通過門檻 | 判定 |",
        "|------|---:|---:|:---:|",
    ]
    checks = [
        ("命中率", overall["hit_rate"], PASS_CRITERIA["hit_rate"], "min"),
        ("誤命中率 ★", overall["false_hit_rate"], PASS_CRITERIA["false_hit_rate"], "max"),
        ("退人工率", overall["defer_rate"], PASS_CRITERIA["defer_rate"], "max"),
    ]
    for label, actual, target, direction in checks:
        if actual is None:
            verdict = "—"
        else:
            ok = actual >= target if direction == "min" else actual <= target
            verdict = "✅" if ok else "❌"
        comparator = "≥" if direction == "min" else "≤"
        lines.append(f"| {label} | {_pct(actual)} | {comparator} {_pct(target)} | {verdict} |")

    bound = false_hit_upper_bound(overall.get("false_hit", 0), overall["n"])
    if bound is not None:
        lines += [
            "",
            f"> ⚠️ **0 次誤命中不等於誤命中率為 0。** n={overall['n']} 時，95% 信心上界為 "
            f"**{_pct(bound)}**（rule of three）。"
            f"{'仍高於 0.5% 門檻，本 PoC 無法證明該門檻成立，需靠 US-9 上線後監測。' if bound > PASS_CRITERIA['false_hit_rate'] else ''}",
        ]

    refusal = summary["refusal_test"]
    if refusal["n"]:
        lines += [
            "",
            "## 拒答測試（沒有章可讀的樣本）",
            "",
            f"無章樣本 **{refusal['n']}** 張：正確回 null **{refusal['correctly_refused']}** 張，"
            f"編了一個日期 **{refusal['invented_a_date']}** 張"
            f"（其中 {refusal['invention_landed_on_a_real_lot']} 張編出來的值命中了在庫批次 = 誤命中）。",
            "",
            "| 照片 | 分層 | 讀到 | 判定 |",
            "|---|---|---|---|",
        ]
        lines += [
            f"| {p['photo_id']} | {p['stratum']} | {p['read'] or '**null** ✅'} | {p['outcome']} |"
            for p in refusal["photos"]
        ]
        lines += [
            "",
            "> 這一節量的是 requirement §2.2 要消滅的失敗模式本身：模型讀不到時傾向編一個合理值。"
            "**回 null 才是正確行為**，所以這些樣本不計入上方三率的分母 —— "
            "把正確拒答算成退人工，等於因為系統照設計運作而扣它分。",
        ]

    lines += ["", "## 按候選集大小分層", "",
              "| 候選集 | 樣本 | 命中率 | 誤命中率 | 退人工率 |", "|---|---:|---:|---:|---:|"]
    lines += [_rate_row(k, v) for k, v in summary["by_candidate_count"].items()]
    lines += ["", "> 候選集越大命中率越低是預期的。若 5+ 批時誤命中率明顯升高，"
              "對策可能是**收貨端減少同料號並存批數**（流程解），不是改辨識。"]

    lines += ["", "## 按拍攝分層", "",
              "| 分層 | 樣本 | 命中率 | 誤命中率 | 退人工率 |", "|---|---:|---:|---:|---:|"]
    lines += [_rate_row(k, v) for k, v in summary["by_stratum"].items()]
    lines += ["", "> 若某一層特別差，解法可能是加一盞燈或規定拍攝距離，比調 prompt 便宜十倍。"]

    if summary["defer_reasons"]:
        lines += ["", "## 退人工原因", "", "| 原因 | 次數 |", "|---|---:|"]
        lines += [f"| `{k}` | {v} |" for k, v in sorted(summary["defer_reasons"].items(), key=lambda kv: -kv[1])]

    if summary["recognition_errors"]:
        lines += ["", f"⚠️ 辨識服務錯誤 {summary['recognition_errors']} 次（已計入退人工）。"]

    lines += ["", "## 逐張結果", "",
              "| 照片 | 分層 | 候選 | 結果 | 真值 | 讀到 | 鎖定 | 信心 | 距離 |",
              "|---|---|---:|---|---|---|---|---:|---:|"]
    marks = {Outcome.HIT: "✅ hit", Outcome.FALSE_HIT: "❌ **false_hit**", Outcome.DEFER: "⚪ defer"}
    for e in evaluations:
        r = e.row()
        dist = "—" if r["distance"] is None else f"{r['distance']:.1f}"
        lines.append(
            f"| {r['photo_id']} | {r['stratum']} | {r['candidates']} | {marks[e.outcome]} | "
            f"{r['truth'] or '（無章）'} | {r['read'] or 'null'} | {r['locked'] or '—'} | "
            f"{r['confidence']:.2f} | {dist} |"
        )
    return "\n".join(lines) + "\n"


def run_sweep(samples: list[Sample], readings: dict[str, Recognition]) -> list[dict]:
    rows: list[dict] = []
    for confidence in (0.0, 0.3, 0.5, 0.7, 0.9):
        for max_distance in (0.5, 1.0, 1.5, 2.0):
            for min_margin in (0.0, 0.5, 1.0, 1.5):
                thresholds = {
                    "confidence_threshold": confidence,
                    "max_distance": max_distance,
                    "min_margin": min_margin,
                }
                rates = summarize(score(samples, readings, **thresholds))["overall"]
                rows.append({**thresholds, **{k: rates[k] for k in ("hit_rate", "false_hit_rate", "defer_rate")}})
    return rows


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--annotations", type=Path, required=True, help="ground-truth CSV")
    parser.add_argument("--images", type=Path, help="directory holding the photos")
    parser.add_argument("--replay", type=Path, help="score a previous run's recognitions instead of calling the API")
    parser.add_argument("--record", type=Path, help="write recognitions here for later replay")
    parser.add_argument("--out", type=Path, default=Path("poc/out"), help="output directory")
    parser.add_argument("--provider", default="gemini", choices=["gemini", "claude"])
    parser.add_argument("--model", default=None, help="provider default if omitted")
    parser.add_argument("--media-resolution", default="high", choices=["low", "medium", "high"],
                        help="gemini only; the stamp is a small region of a large photo")
    parser.add_argument("--no-thinking", action="store_true", help="A/B whether adaptive thinking helps on faint stamps")
    parser.add_argument("--effort", default="high", choices=["low", "medium", "high", "xhigh", "max"])
    parser.add_argument("--confidence-threshold", type=float, default=0.0)
    parser.add_argument("--max-distance", type=float, default=1.5)
    parser.add_argument("--min-margin", type=float, default=1.0)
    parser.add_argument("--sweep", action="store_true", help="re-score across threshold combinations")
    args = parser.parse_args()

    samples = load_samples(args.annotations)
    if not samples:
        print("no samples in annotations file", file=sys.stderr)
        return 1

    if args.replay:
        recorded = json.loads(args.replay.read_text(encoding="utf-8"))
        readings = {k: Recognition(**v) for k, v in recorded.items()}
        print(f"replaying {len(readings)} recognitions from {args.replay}", file=sys.stderr)
    else:
        if not args.images:
            print("--images is required unless --replay is given", file=sys.stderr)
            return 1
        try:
            if args.provider == "gemini":
                model = args.model or "gemini-pro-latest"
                provider = GeminiProvider(
                    model=model,
                    thinking=not args.no_thinking,
                    media_resolution=args.media_resolution,
                )
                detail = f"media_resolution={args.media_resolution}"
            else:
                model = args.model or "claude-opus-5"
                provider = ClaudeProvider(model=model, thinking=not args.no_thinking, effort=args.effort)
                detail = f"effort={args.effort}"
        except RuntimeError as exc:
            print(str(exc), file=sys.stderr)
            return 2
        args.model = model
        print(f"recognising {len(samples)} photos with {model}"
              f" (thinking={'off' if args.no_thinking else 'on'}, {detail})", file=sys.stderr)
        readings = recognize_all(samples, args.images, provider)
        if args.record:
            args.record.parent.mkdir(parents=True, exist_ok=True)
            args.record.write_text(
                json.dumps({k: asdict(v) for k, v in readings.items()}, ensure_ascii=False, indent=2),
                encoding="utf-8",
            )
            print(f"recorded recognitions -> {args.record}", file=sys.stderr)

    thresholds = {
        "confidence_threshold": args.confidence_threshold,
        "max_distance": args.max_distance,
        "min_margin": args.min_margin,
    }
    evaluations = score(samples, readings, **thresholds)
    summary = summarize(evaluations)

    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "summary.json").write_text(
        json.dumps({"thresholds": thresholds, "model": args.model or "replay", **summary}, ensure_ascii=False, indent=2),
        encoding="utf-8",
    )
    with (args.out / "results.csv").open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=list(evaluations[0].row().keys()))
        writer.writeheader()
        writer.writerows(e.row() for e in evaluations)
    report = render_report(summary, thresholds, evaluations, args.model or "replay")
    (args.out / "report.md").write_text(report, encoding="utf-8")

    if args.sweep:
        rows = run_sweep(samples, readings)
        with (args.out / "sweep.csv").open("w", newline="", encoding="utf-8") as handle:
            writer = csv.DictWriter(handle, fieldnames=list(rows[0].keys()))
            writer.writeheader()
            writer.writerows(rows)
        print(f"sweep -> {args.out / 'sweep.csv'} ({len(rows)} combinations)", file=sys.stderr)

    print(report)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
