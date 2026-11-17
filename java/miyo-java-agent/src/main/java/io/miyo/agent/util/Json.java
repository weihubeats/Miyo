package io.miyo.agent.util;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * 极简 JSON 解析 / 序列化（零依赖）。
 *
 * 解析产物用 JDK 自带容器表达：Map&lt;String,Object&gt; / List&lt;Object&gt; /
 * String / Double / Long / Boolean / null——与参数转换、结果序列化共用。
 *
 * 线程安全：纯静态无状态。
 */
public final class Json {

    private Json() {
    }

    // ------------------------------------------------------------------
    // 解析
    // ------------------------------------------------------------------

    /** 解析 JSON 文本；非法输入抛 {@link JsonParseException}。 */
    public static Object parse(String text) {
        Parser p = new Parser(text);
        p.skipWs();
        Object value = p.parseValue();
        p.skipWs();
        if (!p.eof()) {
            throw p.error("JSON 尾部存在多余内容");
        }
        return value;
    }

    static final class JsonParseException extends RuntimeException {
        public JsonParseException(String message) {
            super(message);
        }
    }

    private static final class Parser {
        private final String text;
        private int pos;

        Parser(String text) {
            this.text = text;
        }

        boolean eof() {
            return pos >= text.length();
        }

        JsonParseException error(String message) {
            return new JsonParseException("JSON 解析失败（位置 " + pos + "）：" + message);
        }

        void skipWs() {
            while (!eof()) {
                char c = text.charAt(pos);
                if (c == ' ' || c == '\t' || c == '\n' || c == '\r') {
                    pos++;
                } else {
                    break;
                }
            }
        }

        Object parseValue() {
            if (eof()) {
                throw error("意外的结尾");
            }
            char c = text.charAt(pos);
            switch (c) {
                case '{':
                    return parseObject();
                case '[':
                    return parseArray();
                case '"':
                    return parseString();
                case 't':
                    expect("true");
                    return Boolean.TRUE;
                case 'f':
                    expect("false");
                    return Boolean.FALSE;
                case 'n':
                    expect("null");
                    return null;
                default:
                    return parseNumber();
            }
        }

        private void expect(String literal) {
            if (!text.startsWith(literal, pos)) {
                throw error("非法字面量（期望 " + literal + "）");
            }
            pos += literal.length();
        }

        private Map<String, Object> parseObject() {
            pos++; // {
            Map<String, Object> map = new LinkedHashMap<>();
            skipWs();
            if (!eof() && text.charAt(pos) == '}') {
                pos++;
                return map;
            }
            while (true) {
                skipWs();
                if (eof() || text.charAt(pos) != '"') {
                    throw error("对象键必须是字符串");
                }
                String key = parseString();
                skipWs();
                if (eof() || text.charAt(pos) != ':') {
                    throw error("对象键后缺少 ':'");
                }
                pos++;
                skipWs();
                map.put(key, parseValue());
                skipWs();
                if (eof()) {
                    throw error("对象未闭合");
                }
                char c = text.charAt(pos);
                if (c == ',') {
                    pos++;
                } else if (c == '}') {
                    pos++;
                    return map;
                } else {
                    throw error("对象内应为 ',' 或 '}'");
                }
            }
        }

        private List<Object> parseArray() {
            pos++; // [
            List<Object> list = new ArrayList<>();
            skipWs();
            if (!eof() && text.charAt(pos) == ']') {
                pos++;
                return list;
            }
            while (true) {
                skipWs();
                list.add(parseValue());
                skipWs();
                if (eof()) {
                    throw error("数组未闭合");
                }
                char c = text.charAt(pos);
                if (c == ',') {
                    pos++;
                } else if (c == ']') {
                    pos++;
                    return list;
                } else {
                    throw error("数组内应为 ',' 或 ']'");
                }
            }
        }

        private String parseString() {
            pos++; // "
            StringBuilder sb = new StringBuilder();
            while (!eof()) {
                char c = text.charAt(pos++);
                if (c == '"') {
                    return sb.toString();
                }
                if (c == '\\') {
                    if (eof()) {
                        throw error("转义符后缺少字符");
                    }
                    char esc = text.charAt(pos++);
                    switch (esc) {
                        case '"': sb.append('"'); break;
                        case '\\': sb.append('\\'); break;
                        case '/': sb.append('/'); break;
                        case 'b': sb.append('\b'); break;
                        case 'f': sb.append('\f'); break;
                        case 'n': sb.append('\n'); break;
                        case 'r': sb.append('\r'); break;
                        case 't': sb.append('\t'); break;
                        case 'u':
                            if (pos + 4 > text.length()) {
                                throw error("\\u 转义不完整");
                            }
                            sb.append((char) Integer.parseInt(text.substring(pos, pos + 4), 16));
                            pos += 4;
                            break;
                        default:
                            throw error("未知转义：\\" + esc);
                    }
                } else {
                    sb.append(c);
                }
            }
            throw error("字符串未闭合");
        }

        private Object parseNumber() {
            int start = pos;
            if (!eof() && (text.charAt(pos) == '-' || text.charAt(pos) == '+')) {
                pos++;
            }
            boolean fractional = false;
            while (!eof()) {
                char c = text.charAt(pos);
                if (c >= '0' && c <= '9') {
                    pos++;
                } else if (c == '.' || c == 'e' || c == 'E' || c == '+' || c == '-') {
                    fractional = fractional || c == '.' || c == 'e' || c == 'E';
                    pos++;
                } else {
                    break;
                }
            }
            String raw = text.substring(start, pos);
            if (raw.isEmpty() || raw.equals("-") || raw.equals("+")) {
                throw error("非法数字");
            }
            try {
                if (fractional) {
                    return Double.parseDouble(raw);
                }
                return Long.parseLong(raw);
            } catch (NumberFormatException e) {
                try {
                    return Double.parseDouble(raw);
                } catch (NumberFormatException e2) {
                    throw error("非法数字：" + raw);
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // 序列化（容器 / 标量 → JSON 文本）
    // ------------------------------------------------------------------

    /** 序列化（Map/List/String/Number/Boolean/null；其他类型 toString）。 */
    public static String write(Object value) {
        StringBuilder sb = new StringBuilder(256);
        writeValue(sb, value);
        return sb.toString();
    }

    private static void writeValue(StringBuilder sb, Object value) {
        if (value == null) {
            sb.append("null");
            return;
        }
        if (value instanceof String) {
            writeString(sb, (String) value);
            return;
        }
        if (value instanceof Boolean || value instanceof Long || value instanceof Double
                || value instanceof Integer || value instanceof Float || value instanceof Short
                || value instanceof Byte) {
            sb.append(value);
            return;
        }
        if (value instanceof Map<?, ?>) {
            sb.append('{');
            boolean first = true;
            for (Map.Entry<?, ?> entry : ((Map<?, ?>) value).entrySet()) {
                if (!first) {
                    sb.append(',');
                }
                first = false;
                writeString(sb, String.valueOf(entry.getKey()));
                sb.append(':');
                writeValue(sb, entry.getValue());
            }
            sb.append('}');
            return;
        }
        if (value instanceof List<?>) {
            sb.append('[');
            boolean first = true;
            for (Object item : (List<?>) value) {
                if (!first) {
                    sb.append(',');
                }
                first = false;
                writeValue(sb, item);
            }
            sb.append(']');
            return;
        }
        if (value instanceof Number) {
            sb.append(value);
            return;
        }
        writeString(sb, value.toString());
    }

    private static void writeString(StringBuilder sb, String s) {
        sb.append('"');
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '"': sb.append("\\\""); break;
                case '\\': sb.append("\\\\"); break;
                case '\n': sb.append("\\n"); break;
                case '\r': sb.append("\\r"); break;
                case '\t': sb.append("\\t"); break;
                case '\b': sb.append("\\b"); break;
                case '\f': sb.append("\\f"); break;
                default:
                    if (c < 0x20) {
                        sb.append(String.format("\\u%04x", (int) c));
                    } else {
                        sb.append(c);
                    }
            }
        }
        sb.append('"');
    }
}
