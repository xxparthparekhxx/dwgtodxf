//! Control points for fit-point splines.
//!
//! AutoCAD stores splines drawn through fit points without control points.
//! DXF readers mostly expect control points, so we compute them the way
//! AutoCAD does: cubic global interpolation with chord-length parameters
//! (knots are the cumulative chord lengths) and end tangents.

use crate::bits::Point3;

fn sub(a: Point3, b: Point3) -> Point3 {
    Point3::new(a.x - b.x, a.y - b.y, a.z - b.z)
}
fn add(a: Point3, b: Point3) -> Point3 {
    Point3::new(a.x + b.x, a.y + b.y, a.z + b.z)
}
fn mul(a: Point3, s: f64) -> Point3 {
    Point3::new(a.x * s, a.y * s, a.z * s)
}
fn dist(a: Point3, b: Point3) -> f64 {
    let d = sub(a, b);
    (d.x * d.x + d.y * d.y + d.z * d.z).sqrt()
}
fn unit(a: Point3) -> Point3 {
    let l = dist(a, Point3::default());
    if l == 0.0 {
        a
    } else {
        mul(a, 1.0 / l)
    }
}

/// Non-zero cubic basis functions at `u` for knot span `span` (Piegl & Tiller A2.2).
fn basis(span: usize, u: f64, knots: &[f64]) -> [f64; 4] {
    let mut n = [0.0; 4];
    let mut left = [0.0; 4];
    let mut right = [0.0; 4];
    n[0] = 1.0;
    for j in 1..=3 {
        left[j] = u - knots[span + 1 - j];
        right[j] = knots[span + j] - u;
        let mut saved = 0.0;
        for r in 0..j {
            let den = right[r + 1] + left[j - r];
            let t = if den == 0.0 { 0.0 } else { n[r] / den };
            n[r] = saved + right[r + 1] * t;
            saved = left[j - r] * t;
        }
        n[j] = saved;
    }
    n
}

fn find_span(u: f64, knots: &[f64], n_ctrl: usize) -> usize {
    let n = n_ctrl - 1;
    if u >= knots[n + 1] {
        return n;
    }
    let (mut lo, mut hi) = (3usize, n + 1);
    let mut mid = (lo + hi) / 2;
    while u < knots[mid] || u >= knots[mid + 1] {
        if u < knots[mid] {
            hi = mid;
        } else {
            lo = mid;
        }
        mid = (lo + hi) / 2;
    }
    mid
}

/// Returns (knots, control points) for a cubic interpolating `fit`, or
/// `None` if the input is degenerate.
pub fn fit_to_control(fit: &[Point3], start_tan: Point3, end_tan: Point3) -> Option<(Vec<f64>, Vec<Point3>)> {
    let n = fit.len().checked_sub(1)?;
    if n < 1 {
        return None;
    }
    // chord-length parameters
    let mut u = vec![0.0; n + 1];
    for k in 1..=n {
        u[k] = u[k - 1] + dist(fit[k], fit[k - 1]);
    }
    let total = u[n];
    if total <= 0.0 || u.windows(2).any(|w| w[1] <= w[0]) {
        return None;
    }
    // end derivatives: unit tangents, estimated when not stored
    let est_start = || unit(sub(fit[1], fit[0]));
    let est_end = || unit(sub(fit[n], fit[n - 1]));
    let d0 = if start_tan == Point3::default() { est_start() } else { unit(start_tan) };
    let dn = if end_tan == Point3::default() { est_end() } else { unit(end_tan) };

    // knots: 0,0,0,0, u1..u(n-1), L,L,L,L  -> n+3 control points
    let mut knots = vec![0.0; 4];
    knots.extend_from_slice(&u[1..n]);
    knots.extend_from_slice(&[total; 4]);
    let m = n + 3; // number of control points
    let mut p = vec![Point3::default(); m];
    p[0] = fit[0];
    p[1] = add(fit[0], mul(d0, knots[4] / 3.0));
    p[m - 1] = fit[n];
    p[m - 2] = sub(fit[n], mul(dn, (total - knots[m - 1]) / 3.0));
    if n == 1 {
        return Some((knots, p));
    }
    // interior equations k = 1..n-1: sum_j N_j(u_k) P_j = Q_k, unknowns P2..P(n)
    // each row touches 3 consecutive control points -> tridiagonal system
    let rows = n - 1;
    let (mut a, mut b, mut c) = (vec![0.0; rows], vec![0.0; rows], vec![0.0; rows]);
    let mut rhs = vec![Point3::default(); rows];
    for k in 1..n {
        let span = find_span(u[k], &knots, m);
        let nb = basis(span, u[k], &knots);
        let first = span - 3;
        let mut r = fit[k];
        // row k-1 couples P(k), P(k+1), P(k+2): sub-, main and super-diagonal
        let mut coef = [0.0; 3];
        for (j, w) in nb.iter().enumerate() {
            let idx = first + j;
            if idx < 2 || idx > m - 3 {
                r = sub(r, mul(p[idx], *w));
            } else if idx - k <= 2 {
                coef[idx - k] += *w;
            }
        }
        let row = k - 1;
        // unknown x_i = P(i+2); row k centres on P(k+1) = x_(k-1)
        a[row] = coef[0];
        b[row] = coef[1];
        c[row] = coef[2];
        rhs[row] = r;
    }
    // Thomas algorithm
    for i in 1..rows {
        if b[i - 1] == 0.0 {
            return None;
        }
        let w = a[i] / b[i - 1];
        b[i] -= w * c[i - 1];
        rhs[i] = sub(rhs[i], mul(rhs[i - 1], w));
    }
    if b[rows - 1] == 0.0 {
        return None;
    }
    let mut x = vec![Point3::default(); rows];
    x[rows - 1] = mul(rhs[rows - 1], 1.0 / b[rows - 1]);
    for i in (0..rows - 1).rev() {
        x[i] = mul(sub(rhs[i], mul(x[i + 1], c[i])), 1.0 / b[i]);
    }
    for (i, v) in x.into_iter().enumerate() {
        p[i + 2] = v;
    }
    Some((knots, p))
}
