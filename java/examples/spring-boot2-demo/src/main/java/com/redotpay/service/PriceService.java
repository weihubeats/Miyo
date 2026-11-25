package com.redotpay.service;

/** 接口式业务（真实项目常见：面向接口编程）。 */
public interface PriceService {

    /** 报价（分）。 */
    long quote(String symbol);
}