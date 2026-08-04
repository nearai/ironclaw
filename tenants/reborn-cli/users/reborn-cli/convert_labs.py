#!/usr/bin/env python3
"""Clinical lab value converter: US conventional ↔ SI units."""
import argparse, csv, sys

MW = {"glucose": 180.156, "creatinine": 113.12, "cholesterol": 386.6534}
ROUND = {"sodium": 0, "glucose": 1, "creatinine": 0, "cholesterol": 1}
UNITS_US = {"sodium": "mEq/L", "glucose": "mg/dL", "creatinine": "mg/dL", "cholesterol": "mg/dL"}
UNITS_SI = {"sodium": "mmol/L", "glucose": "mmol/L", "creatinine": "µmol/L", "cholesterol": "mmol/L"}

def us_to_si(analyte, value):
    if analyte == "sodium": raw = value * 1.0
    else: raw = value * (10.0 / MW[analyte])
    return round(raw, ROUND[analyte])

def si_to_us(analyte, value):
    if analyte == "sodium": raw = value * 1.0
    else: raw = value * (MW[analyte] / 10.0)
    return round(raw, ROUND[analyte])

def main():
    parser = argparse.ArgumentParser(description="Clinical lab unit converter")
    for a in ["glucose","creatinine","cholesterol","sodium"]:
        parser.add_argument(f"--{a}", type=float)
    parser.add_argument("--si", action="store_true", help="Input is SI (reverse)")
    parser.add_argument("--stdin", action="store_true", help="Read CSV: analyte,value")
    args = parser.parse_args()

    direction = "si_to_us" if args.si else "us_to_si"

    if args.stdin:
        for row in csv.reader(sys.stdin):
            if not row or row[0].startswith("#"): continue
            a, v = row[0].strip().lower(), float(row[1].strip())
            conv_func = si_to_us if direction == "si_to_us" else us_to_si
            if direction == "us_to_si":
                si = us_to_si(a, v)
                print(f"{a.capitalize()}:  {v} {UNITS_US[a]}  →  {si} {UNITS_SI[a]}")
            else:
                us = si_to_us(a, v)
                print(f"{a.capitalize()}:  {v} {UNITS_SI[a]}  →  {us} {UNITS_US[a]}")
        return

    for analyte in ["sodium","glucose","creatinine","cholesterol"]:
        val = getattr(args, analyte, None)
        if val is None: continue
        if direction == "us_to_si":
            print(f"{analyte.capitalize()}:  {val} {UNITS_US[analyte]}  →  {us_to_si(analyte, val)} {UNITS_SI[analyte]}")
        else:
            print(f"{analyte.capitalize()}:  {val} {UNITS_SI[analyte]}  →  {si_to_us(analyte, val)} {UNITS_US[analyte]}")

if __name__ == "__main__":
    main()