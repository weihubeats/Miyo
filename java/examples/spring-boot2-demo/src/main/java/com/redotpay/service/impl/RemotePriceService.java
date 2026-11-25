package com.redotpay.service.impl;

import com.redotpay.service.PriceService;
import org.springframework.stereotype.Service;

/** `PriceService` 的实现类（类名 ≠ 接口名）。 */
@Service
public class RemotePriceService implements PriceService {

    @Override
    public long quote(String symbol) {
        return symbol == null ? 0L : symbol.length() * 100L;
    }
}