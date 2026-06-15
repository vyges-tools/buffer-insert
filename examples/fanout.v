module top ( a, o1, o2, o3, o4 );
  input a;
  output o1, o2, o3, o4;
  wire w;
  INV u0 ( .A(a), .Y(w) );
  INV u1 ( .A(w), .Y(o1) );
  INV u2 ( .A(w), .Y(o2) );
  INV u3 ( .A(w), .Y(o3) );
  INV u4 ( .A(w), .Y(o4) );
endmodule
