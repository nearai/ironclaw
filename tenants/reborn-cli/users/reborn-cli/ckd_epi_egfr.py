#!/usr/bin/env python3
"""CKD-EPI 2021 race-free eGFR calculator."""
import math

def compute_egfr(sex, age, scr, unit="mg/dL"):
    """Compute eGFR (mL/min/1.73 m²) and CKD stage.

    Parameters
    ----------
    sex : str
        'male' or 'female'
    age : numeric
        Age in years
    scr : numeric
        Serum creatinine value
    unit : str
        'mg/dL' or 'umol/L'

    Returns
    -------
    dict with keys egfr (float), stage (str)
    """
    if unit.lower() == "umol/l":
        scr = scr / 88.42  # convert to mg/dL

    if sex.lower() == "female":
        kappa = 0.7
        alpha = -0.241
        sex_factor = 1.012
    else:
        kappa = 0.9
        alpha = -0.302
        sex_factor = 1.0

    scr_over_kappa = scr / kappa
    min_part = min(scr_over_kappa, 1) ** alpha
    max_part = max(scr_over_kappa, 1) ** (-1.200)
    age_factor = 0.9938 ** age

    egfr = 142 * min_part * max_part * age_factor * sex_factor
    egfr = round(egfr, 1)

    if egfr >= 90:
        stage = "G1"
    elif egfr >= 60:
        stage = "G2"
    elif egfr >= 45:
        stage = "G3a"
    elif egfr >= 30:
        stage = "G3b"
    elif egfr >= 15:
        stage = "G4"
    else:
        stage = "G5"

    return {"egfr": egfr, "stage": stage}


if __name__ == "__main__":
    patients = [
        ("62-year-old female", "female", 62, 1.3),
        ("45-year-old male", "male", 45, 0.9),
        ("78-year-old male", "male", 78, 2.1),
    ]
    print(f"{'Patient':<30} {'eGFR (mL/min/1.73m²)':<25} {'CKD Stage':<10}")
    print("-" * 65)
    for label, sex, age, scr in patients:
        result = compute_egfr(sex, age, scr)
        print(f"{label:<30} {result['egfr']:<25} {result['stage']:<10}")