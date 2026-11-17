package com.example.demo;

import org.springframework.boot.SpringApplication;
import org.springframework.boot.autoconfigure.SpringBootApplication;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.RequestParam;
import org.springframework.web.bind.annotation.RestController;

/**
 * 零侵入验收 demo（Spring Boot 3.2 / JDK 17+）：普通 Boot 应用——
 * 不含任何 Miyo SDK / 注解 / 构建配置改动。
 */
@SpringBootApplication
public class DemoApplication {

    public static void main(String[] args) {
        SpringApplication.run(DemoApplication.class, args);
    }

    /** 周期活动入口：attach 后触发一次请求即可让上下文捕获命中。 */
    @RestController
    static class PingController {
        @GetMapping("/ping")
        String ping(@RequestParam(defaultValue = "world") String name) {
            return "pong: " + name;
        }
    }
}
