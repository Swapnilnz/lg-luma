uniform sampler2D u_texture;
uniform float u_aa;
uniform vec4 u_edge_fade;
uniform vec4 u_edge_fade_x;
// 1 under multiply and screen blending, which need premultiplied colour.
uniform float u_premultiply;
varying PAINT_PRECISION vec2 v_uv;
varying PAINT_PRECISION vec4 v_color;
varying PAINT_PRECISION vec4 v_shape;
varying PAINT_PRECISION vec2 v_params;
// Per-corner radii (top-left, top-right, bottom-right, bottom-left); x < 0: v_params.x for all.
varying PAINT_PRECISION vec4 v_corners;
// Filter mode, then its values. 0 none, 1 grey, 2 sepia, 3 duotone (dark ink in yzw, light
// ink is the colour), 4 pixel (cells per uv unit in yz), 5 blur (uv radius in yz); 10 and up
// patterns (10 + kind, period in y, ink width in z) drawn in the colour.
varying PAINT_PRECISION vec4 v_filter;
varying PAINT_PRECISION float v_logical_y;
varying PAINT_PRECISION float v_logical_x;
float edge_coverage() {
    float edge = 1.0;
    if (u_edge_fade.y > u_edge_fade.x)
        edge = smoothstep(u_edge_fade.x, u_edge_fade.y, v_logical_y) *
            (1.0 - smoothstep(u_edge_fade.z, u_edge_fade.w, v_logical_y));
    if (u_edge_fade_x.y > u_edge_fade_x.x)
        edge *= smoothstep(u_edge_fade_x.x, u_edge_fade_x.y, v_logical_x);
    if (u_edge_fade_x.w > u_edge_fade_x.z)
        edge *= 1.0 - smoothstep(u_edge_fade_x.z, u_edge_fade_x.w, v_logical_x);
    return edge;
}
float luma(vec3 color) {
    return dot(color, vec3(0.299, 0.587, 0.114));
}
// Ink of lines `width` wide every `period`, antialiased over a logical pixel.
float lines(float x, float period, float width) {
    float d = abs(mod(x + period * 0.5, period) - period * 0.5);
    return 1.0 - smoothstep(width * 0.5 - 0.5, width * 0.5 + 0.5, d);
}
float pattern(float kind, vec2 q, vec2 half_size) {
    float period = max(v_filter.y, 1.0);
    float width = v_filter.z;
    if (kind < 0.5) return lines(q.y, period, width);
    if (kind < 1.5) return lines(q.x, period, width);
    if (kind < 2.5) return lines((q.x + q.y) * 0.7071, period, width);
    if (kind < 3.5) return max(lines(q.x, period, width), lines(q.y, period, width));
    if (kind < 4.5) {
        vec2 cell = mod(q + vec2(period * 0.5), vec2(period)) - vec2(period * 0.5);
        return 1.0 - smoothstep(width * 0.5 - 0.5, width * 0.5 + 0.5, length(cell));
    }
    if (kind < 5.5) {
        // Static grain: a cheap hash of the drawable pixel, kept small for mediump floats.
        vec2 p = mod(floor(gl_FragCoord.xy), vec2(61.0));
        float n = fract(p.x * 0.1031 + p.y * 0.11369 + p.x * p.y * 0.0137);
        n = fract(n * (n + 19.19) * 3.1);
        return n * width;
    }
    if (kind < 6.5) return lines(length(q), period, width);
    if (kind < 7.5) {
        float turn = atan(q.y, q.x) / 6.28318 + 0.5;
        return step(fract(turn * period), width);
    }
    // A floor grid to a horizon at the top edge: rows close up with distance, columns
    // converge on the centre.
    float t = clamp(q.y / max(half_size.y, 1.0) * 0.5 + 0.5, 0.0, 1.0);
    float depth = 1.0 / (t + 0.06);
    float f = fract(depth * 0.5);
    float across = min(f, 1.0 - f) / 0.5 / (depth * depth) * 2.0 * half_size.y;
    float column = q.x * depth / max(half_size.x, 1.0) * period * 0.5;
    float g = fract(column);
    float down = min(g, 1.0 - g) / (period * 0.5) * max(half_size.x, 1.0) / depth;
    float ink = max(1.0 - smoothstep(width * 0.5 - 0.5, width * 0.5 + 0.5, across),
                    1.0 - smoothstep(width * 0.5 - 0.5, width * 0.5 + 0.5, down));
    // Fades out toward the horizon, where the lines would crowd into a solid band.
    return ink * smoothstep(0.02, 0.18, t);
}
vec4 shade() {
    float mode = v_filter.x;
    if (mode > 9.5) {
        float ink = pattern(mode - 10.0, v_shape.xy, v_shape.zw);
        return vec4(v_color.rgb, v_color.a * ink);
    }
    vec2 uv = v_uv;
    if (mode > 3.5 && mode < 4.5)
        uv = (floor(uv * v_filter.yz) + 0.5) / v_filter.yz;
    vec4 color;
    if (mode > 4.5 && mode < 5.5) {
        // Twelve taps on two rings and the centre: soft enough for a still backdrop.
        vec2 r = v_filter.yz;
        color = texture2D(u_texture, uv) * 0.16;
        color += texture2D(u_texture, uv + r * vec2(1.0, 0.0)) * 0.07;
        color += texture2D(u_texture, uv + r * vec2(-1.0, 0.0)) * 0.07;
        color += texture2D(u_texture, uv + r * vec2(0.0, 1.0)) * 0.07;
        color += texture2D(u_texture, uv + r * vec2(0.0, -1.0)) * 0.07;
        color += texture2D(u_texture, uv + r * vec2(0.7, 0.7)) * 0.07;
        color += texture2D(u_texture, uv + r * vec2(-0.7, 0.7)) * 0.07;
        color += texture2D(u_texture, uv + r * vec2(0.7, -0.7)) * 0.07;
        color += texture2D(u_texture, uv + r * vec2(-0.7, -0.7)) * 0.07;
        color += texture2D(u_texture, uv + r * vec2(0.5, 0.0)) * 0.07;
        color += texture2D(u_texture, uv + r * vec2(-0.5, 0.0)) * 0.07;
        color += texture2D(u_texture, uv + r * vec2(0.0, 0.5)) * 0.06;
        color += texture2D(u_texture, uv + r * vec2(0.0, -0.5)) * 0.06;
    } else {
        color = texture2D(u_texture, uv);
    }
    if (mode > 0.5 && mode < 1.5) {
        color.rgb = vec3(luma(color.rgb));
    } else if (mode > 1.5 && mode < 2.5) {
        color.rgb = clamp(vec3(dot(color.rgb, vec3(0.393, 0.769, 0.189)),
                               dot(color.rgb, vec3(0.349, 0.686, 0.168)),
                               dot(color.rgb, vec3(0.272, 0.534, 0.131))), 0.0, 1.0);
    } else if (mode > 2.5 && mode < 3.5) {
        return vec4(mix(v_filter.yzw, v_color.rgb, luma(color.rgb)), color.a * v_color.a);
    }
    return color * v_color;
}
vec4 finish(vec4 color) {
    color.a *= edge_coverage();
    if (u_premultiply > 0.5)
        color.rgb *= color.a;
    return color;
}
void main() {
    if (v_params.x < 0.0) {
        vec4 color = shade();
        // -2: wallpaper with folded full-screen fills, p * m + k (k in v_shape).
        if (v_params.x < -1.5)
            color.rgb += v_shape.rgb;
        gl_FragColor = finish(color);
        return;
    }
    float blur = max(-v_params.y, 0.0);
    vec2 half_size = v_shape.zw - vec2(blur);
    float corner = v_params.x;
    if (v_corners.x >= 0.0)
        corner = v_shape.x < 0.0 ? (v_shape.y < 0.0 ? v_corners.x : v_corners.w)
                                 : (v_shape.y < 0.0 ? v_corners.y : v_corners.z);
    float radius = min(corner, min(half_size.x, half_size.y));
    vec2 q = abs(v_shape.xy) - half_size + vec2(radius);
    float d = length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - radius;
    // Box-filtered coverage of one drawable pixel (u_aa) centred on the edge: an edge on a
    // pixel boundary stays one crisp step, and a one-pixel border is fully opaque.
    float alpha = clamp(0.5 - d / u_aa, 0.0, 1.0);
    if (blur > 0.0) {
        alpha = 1.0 - smoothstep(-blur, blur, d);
    }
    if (v_params.y > 0.0) {
        alpha *= clamp(0.5 + (d + v_params.y) / u_aa, 0.0, 1.0);
    }
    vec4 color = shade();
    gl_FragColor = finish(vec4(color.rgb, color.a * alpha));
}
