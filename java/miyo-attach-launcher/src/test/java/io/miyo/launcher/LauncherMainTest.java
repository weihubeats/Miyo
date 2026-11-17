package io.miyo.launcher;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import java.io.ByteArrayInputStream;
import java.nio.charset.StandardCharsets;
import java.util.LinkedHashMap;
import java.util.Map;

import org.junit.Test;

/**
 * Launcher 参数解析单测（不依赖 Attach 工具链）：
 * 覆盖 --args-stdin（宿主现行路径，token 不进命令行）与 --args 兼容路径。
 */
public class LauncherMainTest {

    @Test
    public void readAllReadsFullStreamAsUtf8() throws Exception {
        String payload = "token=abc123,infoFile=/tmp/x.json,expectedPid=42";
        byte[] bytes = payload.getBytes(StandardCharsets.UTF_8);
        // 分块喂入（越过单次 read 即返回的伪实现）
        java.io.SequenceInputStream seq = new java.io.SequenceInputStream(
                new ByteArrayInputStream(bytes, 0, 10),
                new ByteArrayInputStream(bytes, 10, bytes.length - 10));
        assertEquals(payload, LauncherMain.readAll(seq));
        assertEquals("", LauncherMain.readAll(new ByteArrayInputStream(new byte[0])));
    }

    @Test
    public void readAgentArgsPrefersStdin() throws Exception {
        Map<String, String> opts = new LinkedHashMap<>();
        opts.put("args-stdin", "");
        opts.put("args", "token=from-argv");
        String stdinPayload = "token=from-stdin,maxConcurrent=4";
        String args = readWithStdin(opts, stdinPayload);
        assertEquals(stdinPayload, args);
    }

    @Test
    public void readAgentArgsFallsBackToArgvCompat() throws Exception {
        Map<String, String> opts = new LinkedHashMap<>();
        opts.put("args", "token=from-argv");
        String args = readWithStdin(opts, "");
        assertEquals("token=from-argv", args);
    }

    @Test
    public void readAgentArgsMissingBothIsEmpty() throws Exception {
        String args = readWithStdin(new LinkedHashMap<String, String>(), "");
        assertEquals("", args);
    }

    /** 替换 System.in 后调用包级 readAgentArgs（attach 路径的参数解析内核）。 */
    private static String readWithStdin(Map<String, String> opts, String stdinPayload)
            throws Exception {
        java.io.InputStream originalIn = System.in;
        System.setIn(new ByteArrayInputStream(stdinPayload.getBytes(StandardCharsets.UTF_8)));
        try {
            return LauncherMain.readAgentArgs(opts);
        } finally {
            System.setIn(originalIn);
        }
    }
}
