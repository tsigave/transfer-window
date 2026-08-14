pub(crate) type Vector3 = [f64; 3];

pub(crate) fn add(left: Vector3, right: Vector3) -> Vector3 {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

pub(crate) fn sub(left: Vector3, right: Vector3) -> Vector3 {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

pub(crate) fn scale(vector: Vector3, factor: f64) -> Vector3 {
    [vector[0] * factor, vector[1] * factor, vector[2] * factor]
}

pub(crate) fn dot(left: Vector3, right: Vector3) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

pub(crate) fn norm(vector: Vector3) -> f64 {
    dot(vector, vector).sqrt()
}

pub(crate) fn unit(vector: Vector3) -> Option<Vector3> {
    let magnitude = norm(vector);
    (magnitude.is_finite() && magnitude > 0.0).then(|| scale(vector, 1.0 / magnitude))
}
