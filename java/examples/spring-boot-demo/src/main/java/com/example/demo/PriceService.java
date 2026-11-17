package com.example.demo;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.concurrent.ThreadLocalRandom;

import org.springframework.stereotype.Service;

/** 普通业务 Bean（Bean 探测 / Bean 方法调用验收目标）。 */
@Service
public class PriceService {

    /** Bean 方法：基本类型 + Map 返回值。 */
    public Map<String, Object> quote(String sku, int quantity, double discount) {
        if (discount < 0 || discount > 1) {
            throw new IllegalArgumentException("折扣必须在 0..1：" + discount);
        }
        double unit = 100 + ThreadLocalRandom.current().nextInt(900);
        double total = unit * quantity * (1 - discount);
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("sku", sku);
        out.put("quantity", quantity);
        out.put("unit", unit);
        out.put("total", Math.round(total * 100) / 100.0);
        return out;
    }

    public int beanHashCodeForIdentity() {
        return System.identityHashCode(this);
    }
}
